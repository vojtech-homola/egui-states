//! Read-only client mirror of a server-controlled vector.

use parking_lot::RwLock;
use std::sync::Arc;

use serde::Deserialize;

use crate::protocol::collections::{Part, Replacement, VecHeader};
use crate::protocol::{Deserializer, deserialize};
use crate::typed::Typed;

pub(crate) trait UpdateList: Sync + Send {
    fn update_list(&self, type_id: u32, header: VecHeader, data: &[u8]) -> Result<(), String>;
    /// Drops a chunked replacement cut short by a closed connection.
    fn disconnected(&self);
}

/// A server-controlled vector mirrored read-only on the client.
///
/// Complete replacements and item-level changes arrive from the server; the
/// client exposes only snapshot and borrowing accessors.
pub struct VecState<T> {
    name: String,
    type_id: u32,
    list: Arc<RwLock<Vec<T>>>,
    // Items of a chunked replacement received so far; applied on `End`.
    pending: Arc<Replacement<Vec<T>>>,
}

impl<T: Typed + Clone> VecState<T> {
    pub(crate) fn new(name: String, type_id: u32) -> Self {
        Self {
            name,
            type_id,
            list: Arc::new(RwLock::new(Vec::new())),
            pending: Arc::new(Replacement::new()),
        }
    }

    /// Returns a copy of the complete vector.
    pub fn get(&self) -> Vec<T> {
        self.list.read().clone()
    }

    /// Returns a copy of the item at `idx`, if it exists.
    pub fn get_item(&self, idx: usize) -> Option<T> {
        self.list.read().get(idx).cloned()
    }

    /// Borrows the complete vector for the duration of `f`.
    ///
    /// The synchronization read lock remains held while `f` runs.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn read<R>(&self, mut f: impl FnMut(&Vec<T>) -> R) -> R {
        let l = self.list.read();
        f(&*l)
    }

    /// Borrows the item at `idx` for the duration of `f`.
    ///
    /// The synchronization read lock remains held while `f` runs.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn read_item<R>(&self, idx: usize, mut f: impl FnMut(Option<&T>) -> R) -> R {
        let l = self.list.read();
        f(l.get(idx))
    }
}

impl<T: for<'a> Deserialize<'a>> VecState<T> {
    fn decode_items(&self, size: u64, data: &[u8], list: &mut Vec<T>) -> Result<(), String> {
        let mut deserializer = Deserializer::new(data);
        // `size` comes from the network, so the capacity is capped by the payload length.
        list.reserve((size as usize).min(data.len()));
        for _ in 0..size {
            let item: T = deserializer
                .get()
                .map_err(|e| format!("Error deserializing list item for {}: {}", self.name, e))?;
            list.push(item);
        }
        Ok(())
    }

    /// Applies one part of a whole-list replacement.
    fn replace(&self, part: Part, size: u64, data: &[u8]) -> Result<(), String> {
        let complete = self.pending.apply(
            part,
            size,
            data,
            |size, data, list| self.decode_items(size, data, list),
            || format!("List chunk without a beginning for {}", self.name),
        )?;
        if let Some(list) = complete {
            // The previous list is dropped after the lock is released.
            let _previous = std::mem::replace(&mut *self.list.write(), list);
        }
        Ok(())
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateList for VecState<T> {
    fn update_list(&self, type_id: u32, header: VecHeader, data: &[u8]) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for list {}", self.name));
        }

        match header {
            VecHeader::All(size) => self.replace(Part::All, size, data)?,
            VecHeader::Begin(size) => self.replace(Part::Begin, size, data)?,
            VecHeader::Chunk(size) => self.replace(Part::Chunk, size, data)?,
            VecHeader::End(size) => self.replace(Part::End, size, data)?,
            VecHeader::Set(idx) => {
                let value: T = deserialize(data).map_err(|e| {
                    format!("Error deserializing list item for {}: {}", self.name, e)
                })?;
                let mut list = self.list.write();
                let idx = idx as usize;
                if idx < list.len() {
                    list[idx] = value;
                }
            }
            VecHeader::Add => {
                let value: T = deserialize(data).map_err(|e| {
                    format!("Error deserializing list item for {}: {}", self.name, e)
                })?;
                self.list.write().push(value);
            }
            VecHeader::Remove(idx) => {
                let mut list = self.list.write();
                let idx = idx as usize;
                if idx < list.len() {
                    list.remove(idx);
                }
            }
        }
        Ok(())
    }

    fn disconnected(&self) {
        self.pending.clear();
    }
}

impl<T> Clone for VecState<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            type_id: self.type_id,
            list: self.list.clone(),
            pending: self.pending.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPE_ID: u32 = 5;

    fn items(values: &[i32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| postcard::to_stdvec(value).unwrap())
            .collect()
    }

    fn list(values: &[i32]) -> VecState<i32> {
        let list = VecState::new("root.list".into(), TYPE_ID);
        list.update_list(TYPE_ID, VecHeader::All(values.len() as u64), &items(values))
            .unwrap();
        list
    }

    #[test]
    fn replace_set_add_and_remove_apply_in_order() {
        let list = list(&[1, 2, 3]);
        assert_eq!(list.get(), [1, 2, 3]);
        list.update_list(TYPE_ID, VecHeader::Set(1), &items(&[20]))
            .unwrap();
        list.update_list(TYPE_ID, VecHeader::Add, &items(&[4]))
            .unwrap();
        list.update_list(TYPE_ID, VecHeader::Remove(0), &[])
            .unwrap();
        assert_eq!(list.get(), [20, 3, 4]);
        assert_eq!(list.get_item(2), Some(4));
        assert_eq!(list.get_item(3), None);
        list.update_list(TYPE_ID, VecHeader::All(0), &[]).unwrap();
        assert!(list.get().is_empty());
    }

    /// Pins current behavior: the server validates indexes, so the client ignores
    /// out-of-range item updates instead of reporting them.
    #[test]
    fn out_of_range_set_and_remove_are_ignored() {
        let list = list(&[1, 2]);
        list.update_list(TYPE_ID, VecHeader::Set(2), &items(&[9]))
            .unwrap();
        list.update_list(TYPE_ID, VecHeader::Remove(2), &[])
            .unwrap();
        assert_eq!(list.get(), [1, 2]);
    }

    #[test]
    fn mismatched_type_and_malformed_items_are_rejected_without_changes() {
        let list = list(&[1, 2]);
        assert!(
            list.update_list(TYPE_ID + 1, VecHeader::Add, &items(&[3]))
                .is_err()
        );
        assert!(list.update_list(TYPE_ID, VecHeader::Add, &[]).is_err());
        assert!(list.update_list(TYPE_ID, VecHeader::Set(0), &[]).is_err());
        assert_eq!(list.get(), [1, 2]);
    }

    /// A replacement whose payload ends early must not leave a partial list.
    #[test]
    fn malformed_replacement_keeps_the_previous_list() {
        let list = list(&[1, 2]);
        assert!(
            list.update_list(TYPE_ID, VecHeader::All(3), &items(&[7, 8]))
                .is_err()
        );
        assert_eq!(list.get(), [1, 2]);
    }

    /// A chunked replacement becomes visible only on `End`, all at once.
    #[test]
    fn chunked_replacement_applies_on_end() {
        let list = list(&[1, 2]);
        list.update_list(TYPE_ID, VecHeader::Begin(2), &items(&[3, 4]))
            .unwrap();
        list.update_list(TYPE_ID, VecHeader::Chunk(1), &items(&[5]))
            .unwrap();
        assert_eq!(list.get(), [1, 2]);
        list.update_list(TYPE_ID, VecHeader::End(1), &items(&[6]))
            .unwrap();
        assert_eq!(list.get(), [3, 4, 5, 6]);
    }

    #[test]
    fn chunks_without_a_beginning_are_rejected_without_changes() {
        let list = list(&[1, 2]);
        assert!(
            list.update_list(TYPE_ID, VecHeader::Chunk(1), &items(&[3]))
                .is_err()
        );
        assert!(
            list.update_list(TYPE_ID, VecHeader::End(1), &items(&[3]))
                .is_err()
        );
        assert_eq!(list.get(), [1, 2]);
    }

    /// A malformed chunk discards the transfer; later chunks of it are rejected.
    #[test]
    fn malformed_chunk_discards_the_replacement() {
        let list = list(&[1, 2]);
        list.update_list(TYPE_ID, VecHeader::Begin(1), &items(&[3]))
            .unwrap();
        assert!(
            list.update_list(TYPE_ID, VecHeader::Chunk(2), &items(&[4]))
                .is_err()
        );
        assert!(
            list.update_list(TYPE_ID, VecHeader::End(1), &items(&[5]))
                .is_err()
        );
        assert_eq!(list.get(), [1, 2]);
    }

    /// A transfer cut short (for example by a reconnect) is replaced by the next one.
    #[test]
    fn a_new_beginning_replaces_an_unfinished_replacement() {
        let list = list(&[1]);
        list.update_list(TYPE_ID, VecHeader::Begin(1), &items(&[2]))
            .unwrap();
        list.update_list(TYPE_ID, VecHeader::Begin(1), &items(&[3]))
            .unwrap();
        list.update_list(TYPE_ID, VecHeader::End(1), &items(&[4]))
            .unwrap();
        assert_eq!(list.get(), [3, 4]);
    }

    /// An interrupted transfer must not keep its items: a disconnect or a later
    /// complete replacement releases them, so its remaining chunks are rejected.
    #[test]
    fn disconnect_and_full_replacement_release_an_interrupted_replacement() {
        let list = list(&[1]);
        list.update_list(TYPE_ID, VecHeader::Begin(1), &items(&[2]))
            .unwrap();
        list.disconnected();
        assert!(
            list.update_list(TYPE_ID, VecHeader::End(1), &items(&[3]))
                .is_err()
        );

        list.update_list(TYPE_ID, VecHeader::Begin(1), &items(&[2]))
            .unwrap();
        list.update_list(TYPE_ID, VecHeader::All(0), &[]).unwrap();
        assert!(!list.pending.is_pending());
        assert!(
            list.update_list(TYPE_ID, VecHeader::End(1), &items(&[3]))
                .is_err()
        );
        assert!(list.get().is_empty());
    }
}
