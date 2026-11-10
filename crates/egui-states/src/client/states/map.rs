//! Read-only client mirror of a server-controlled map.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use serde::Deserialize;

use crate::protocol::collections::{MapHeader, Part, Replacement};
use crate::protocol::{Deserializer, deserialize};
use crate::typed::Typed;

pub(crate) trait UpdateMap: Sync + Send {
    fn update_map(&self, type_id: u32, header: MapHeader, data: &[u8]) -> Result<(), String>;
    /// Drops a chunked replacement cut short by a closed connection.
    fn disconnected(&self);
}

/// A server-controlled hash map mirrored read-only on the client.
///
/// Complete replacements, insertions, and removals arrive from the server; the
/// client exposes only snapshot and borrowing accessors.
pub struct MapState<K, V> {
    name: String,
    type_id: u32,
    dict: Arc<RwLock<HashMap<K, V>>>,
    // Items of a chunked replacement received so far; applied on `End`.
    pending: Arc<Replacement<HashMap<K, V>>>,
}

impl<K, V> MapState<K, V>
where
    K: Typed + Clone + Hash + Eq,
    V: Typed + Clone,
{
    pub(crate) fn new(name: String, type_id: u32) -> Self {
        Self {
            name,
            type_id,
            dict: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(Replacement::new()),
        }
    }

    /// Returns a copy of the complete map.
    #[inline]
    pub fn get(&self) -> HashMap<K, V> {
        self.dict.read().clone()
    }

    /// Returns a copy of the value at `key`, if it exists.
    #[inline]
    pub fn get_item(&self, key: &K) -> Option<V> {
        self.dict.read().get(key).cloned()
    }

    /// Borrows the complete map for the duration of `f`.
    ///
    /// The synchronization read lock remains held while `f` runs.
    pub fn read<R>(&self, mut f: impl FnMut(&HashMap<K, V>) -> R) -> R {
        let d = self.dict.read();
        f(&*d)
    }

    /// Borrows the value at `key` for the duration of `f`.
    ///
    /// The synchronization read lock remains held while `f` runs.
    pub fn read_item<R>(&self, key: &K, mut f: impl FnMut(Option<&V>) -> R) -> R {
        let d = self.dict.read();
        let v = d.get(key);
        f(v)
    }
}

impl<K, V> MapState<K, V>
where
    K: for<'a> Deserialize<'a> + Eq + Hash,
    V: for<'a> Deserialize<'a>,
{
    fn decode_items(&self, size: u64, data: &[u8], map: &mut HashMap<K, V>) -> Result<(), String> {
        let mut deserializer = Deserializer::new(data);
        // `size` comes from the network, so the capacity is capped by the payload length.
        map.reserve((size as usize).min(data.len()));
        for _ in 0..size {
            let key: K = deserializer
                .get()
                .map_err(|e| format!("Error deserializing dict key for {}: {}", self.name, e))?;
            let value: V = deserializer
                .get()
                .map_err(|e| format!("Error deserializing dict value for {}: {}", self.name, e))?;
            map.insert(key, value);
        }
        Ok(())
    }

    /// Applies one part of a whole-map replacement.
    fn replace(&self, part: Part, size: u64, data: &[u8]) -> Result<(), String> {
        let complete = self.pending.apply(
            part,
            size,
            data,
            |size, data, map| self.decode_items(size, data, map),
            || format!("Map chunk without a beginning for {}", self.name),
        )?;
        if let Some(map) = complete {
            // The previous map is dropped after the lock is released.
            let _previous = std::mem::replace(&mut *self.dict.write(), map);
        }
        Ok(())
    }
}

impl<K, V> UpdateMap for MapState<K, V>
where
    K: for<'a> Deserialize<'a> + Eq + Hash + Send + Sync,
    V: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_map(&self, type_id: u32, header: MapHeader, data: &[u8]) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for map {}", self.name));
        }

        match header {
            MapHeader::All(size) => self.replace(Part::All, size, data)?,
            MapHeader::Begin(size) => self.replace(Part::Begin, size, data)?,
            MapHeader::Chunk(size) => self.replace(Part::Chunk, size, data)?,
            MapHeader::End(size) => self.replace(Part::End, size, data)?,
            MapHeader::Set => {
                let (key, value): (K, V) = deserialize(data).map_err(|e| {
                    format!("Error deserializing dict item for {}: {}", self.name, e)
                })?;
                self.dict.write().insert(key, value);
            }
            MapHeader::Remove => {
                let key: K = deserialize(data).map_err(|e| {
                    format!("Error deserializing dict key for {}: {}", self.name, e)
                })?;
                self.dict.write().remove(&key);
            }
        }
        Ok(())
    }

    fn disconnected(&self) {
        self.pending.clear();
    }
}

impl<K, V> Clone for MapState<K, V> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            type_id: self.type_id,
            dict: self.dict.clone(),
            pending: self.pending.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPE_ID: u32 = 6;

    fn entries(values: &[(u16, String)]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|(key, value)| {
                let mut bytes = postcard::to_stdvec(key).unwrap();
                bytes.extend(postcard::to_stdvec(value).unwrap());
                bytes
            })
            .collect()
    }

    fn entry(key: u16, value: &str) -> (u16, String) {
        (key, value.to_string())
    }

    fn map(values: &[(u16, String)]) -> MapState<u16, String> {
        let map = MapState::new("root.map".into(), TYPE_ID);
        map.update_map(
            TYPE_ID,
            MapHeader::All(values.len() as u64),
            &entries(values),
        )
        .unwrap();
        map
    }

    #[test]
    fn replace_insert_overwrite_and_remove_apply_in_order() {
        let map = map(&[entry(1, "one"), entry(2, "two")]);
        map.update_map(TYPE_ID, MapHeader::Set, &entries(&[entry(3, "three")]))
            .unwrap();
        map.update_map(TYPE_ID, MapHeader::Set, &entries(&[entry(1, "uno")]))
            .unwrap();
        map.update_map(
            TYPE_ID,
            MapHeader::Remove,
            &postcard::to_stdvec(&2u16).unwrap(),
        )
        .unwrap();
        assert_eq!(
            map.get(),
            HashMap::from([entry(1, "uno"), entry(3, "three")])
        );
        // Removing a missing key is harmless.
        map.update_map(
            TYPE_ID,
            MapHeader::Remove,
            &postcard::to_stdvec(&9u16).unwrap(),
        )
        .unwrap();
        assert_eq!(map.get_item(&3).as_deref(), Some("three"));
        map.update_map(TYPE_ID, MapHeader::All(0), &[]).unwrap();
        assert!(map.get().is_empty());
    }

    #[test]
    fn mismatched_type_and_malformed_entries_are_rejected_without_changes() {
        let map = map(&[entry(1, "one")]);
        assert!(
            map.update_map(TYPE_ID + 1, MapHeader::Set, &entries(&[entry(2, "two")]))
                .is_err()
        );
        assert!(
            map.update_map(
                TYPE_ID,
                MapHeader::Set,
                &postcard::to_stdvec(&2u16).unwrap()
            )
            .is_err()
        );
        assert!(map.update_map(TYPE_ID, MapHeader::Remove, &[]).is_err());
        assert_eq!(map.get(), HashMap::from([entry(1, "one")]));
    }

    /// A replacement whose payload ends early must not leave a partial map.
    #[test]
    fn malformed_replacement_keeps_the_previous_map() {
        let map = map(&[entry(1, "one")]);
        assert!(
            map.update_map(TYPE_ID, MapHeader::All(2), &entries(&[entry(5, "five")]))
                .is_err()
        );
        assert_eq!(map.get(), HashMap::from([entry(1, "one")]));
    }

    /// A chunked replacement becomes visible only on `End`, all at once.
    #[test]
    fn chunked_replacement_applies_on_end() {
        let map = map(&[entry(1, "one")]);
        map.update_map(TYPE_ID, MapHeader::Begin(1), &entries(&[entry(2, "two")]))
            .unwrap();
        map.update_map(TYPE_ID, MapHeader::Chunk(1), &entries(&[entry(3, "three")]))
            .unwrap();
        assert_eq!(map.get(), HashMap::from([entry(1, "one")]));
        map.update_map(TYPE_ID, MapHeader::End(1), &entries(&[entry(4, "four")]))
            .unwrap();
        assert_eq!(
            map.get(),
            HashMap::from([entry(2, "two"), entry(3, "three"), entry(4, "four")])
        );
    }

    #[test]
    fn chunks_without_a_beginning_are_rejected_without_changes() {
        let map = map(&[entry(1, "one")]);
        let chunk = entries(&[entry(2, "two")]);
        assert!(
            map.update_map(TYPE_ID, MapHeader::Chunk(1), &chunk)
                .is_err()
        );
        assert!(map.update_map(TYPE_ID, MapHeader::End(1), &chunk).is_err());
        assert_eq!(map.get(), HashMap::from([entry(1, "one")]));
    }

    /// An interrupted transfer must not keep its items: a disconnect or a later
    /// complete replacement releases them.
    #[test]
    fn disconnect_and_full_replacement_release_an_interrupted_replacement() {
        let map = map(&[entry(1, "one")]);
        let chunk = entries(&[entry(2, "two")]);
        map.update_map(TYPE_ID, MapHeader::Begin(1), &chunk)
            .unwrap();
        map.disconnected();
        assert!(!map.pending.is_pending());

        map.update_map(TYPE_ID, MapHeader::Begin(1), &chunk)
            .unwrap();
        map.update_map(TYPE_ID, MapHeader::All(0), &[]).unwrap();
        assert!(!map.pending.is_pending());
        assert!(map.update_map(TYPE_ID, MapHeader::End(1), &chunk).is_err());
        assert!(map.get().is_empty());
    }
}
