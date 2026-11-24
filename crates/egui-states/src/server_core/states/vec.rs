use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;

use crate::protocol::collections::{VecHeader, serialize_replacement};
use crate::protocol::{ServerHeader, check_value_size, serialize};
use crate::server_core::net::sender::{MessageSender, SenderData};
use crate::server_core::server::SyncTrait;

/// An index into a list. It is resolved against the length of the list under
/// the list's own lock, so the length cannot change between resolving the
/// index and using it.
pub(crate) trait ListIndex: Copy {
    /// The position of an existing item of a list of `len` items, `None`
    /// when the index is out of range.
    fn item(self, len: usize) -> Option<usize>;

    /// The position of an insert into a list of `len` items, which may be
    /// `len` itself; `None` when the index is out of range.
    fn insert(self, len: usize) -> Option<usize>;
}

impl ListIndex for usize {
    fn item(self, len: usize) -> Option<usize> {
        (self < len).then_some(self)
    }

    fn insert(self, len: usize) -> Option<usize> {
        (self <= len).then_some(self)
    }
}

/// A Python list index, where a negative index counts from the end. An
/// insert past either end is out of range instead of being clamped as
/// `list.insert` does, as for the other operations.
impl ListIndex for isize {
    fn item(self, len: usize) -> Option<usize> {
        from_end(self, len).filter(|&index| index < len)
    }

    fn insert(self, len: usize) -> Option<usize> {
        from_end(self, len).filter(|&index| index <= len)
    }
}

/// The position `index` names in a list of `len` items, not yet checked
/// against the end; `None` when a negative index reaches before the start.
fn from_end(index: isize, len: usize) -> Option<usize> {
    if index < 0 {
        len.checked_sub(index.unsigned_abs())
    } else {
        Some(index.unsigned_abs())
    }
}

pub(crate) struct ValueList {
    // #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    list: RwLock<Vec<Bytes>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueList {
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            list: RwLock::new(Vec::new()),
            sender,
            connected,
        })
    }

    /// Serializes a whole-list replacement, split into frame-sized messages.
    fn serialize_all(&self, vec: &[Bytes]) -> Result<Vec<SenderData>, &'static str> {
        serialize_replacement(
            vec.iter(),
            |item| item.len(),
            |message, item| message.extend_from_slice(item),
            |part, count, size| {
                ServerHeader::ValueVec(self.id, self.type_id, VecHeader::part(part, count), size)
            },
        )
    }

    fn send_all(&self, messages: Vec<SenderData>) {
        for message in messages {
            self.sender.send(message);
        }
    }

    pub(crate) fn set(&self, list: Vec<Bytes>) -> Result<(), String> {
        // the limit applies to single items, not to the whole list
        for value in list.iter() {
            check_value_size(&self.name, value.len())?;
        }

        let mut w = self.list.write();

        if self.connected.load(Ordering::Acquire) {
            self.send_all(self.serialize_all(&list)?);
        }

        *w = list;
        Ok(())
    }

    pub(crate) fn get(&self) -> Vec<Bytes> {
        self.list.read().clone()
    }

    pub(crate) fn set_item_py(&self, idx: impl ListIndex, value: Bytes) -> Result<(), String> {
        check_value_size(&self.name, value.len())?;

        let mut w = self.list.write();
        let Some(idx) = idx.item(w.len()) else {
            return Err("Index out of bounds".to_string());
        };

        if self.connected.load(Ordering::Acquire) {
            let header = ServerHeader::ValueVec(
                self.id,
                self.type_id,
                VecHeader::Set(idx as u64),
                value.len() as u32,
            );
            let mut message = serialize(&header).map_err(|_| "Serialization error")?;
            message.extend_from_slice(&value);
            self.sender.send(message);
        }

        w[idx] = value;
        Ok(())
    }

    pub(crate) fn get_item(&self, idx: impl ListIndex) -> Result<Bytes, &'static str> {
        let r = self.list.read();
        let Some(idx) = idx.item(r.len()) else {
            return Err("Index out of bounds");
        };
        Ok(r[idx].clone())
    }

    pub(crate) fn len(&self) -> usize {
        self.list.read().len()
    }

    pub(crate) fn remove_item(&self, idx: impl ListIndex) -> Result<Bytes, &'static str> {
        let mut w = self.list.write();
        let Some(idx) = idx.item(w.len()) else {
            return Err("Index out of bounds");
        };
        let value = w.remove(idx);

        if self.connected.load(Ordering::Acquire) {
            let header =
                ServerHeader::ValueVec(self.id, self.type_id, VecHeader::Remove(idx as u64), 0);
            let message = serialize(&header).map_err(|_| "Serialization error")?;
            self.sender.send(message);
        }

        Ok(value)
    }

    /// Inserts `value` at `idx`, which may be the length to append.
    pub(crate) fn insert_item(&self, idx: impl ListIndex, value: Bytes) -> Result<(), String> {
        check_value_size(&self.name, value.len())?;

        let mut w = self.list.write();
        let Some(idx) = idx.insert(w.len()) else {
            return Err("Index out of bounds".to_string());
        };
        if self.connected.load(Ordering::Acquire) {
            let header = ServerHeader::ValueVec(
                self.id,
                self.type_id,
                VecHeader::Insert(idx as u64),
                value.len() as u32,
            );
            let mut message = serialize(&header).map_err(|_| "Serialization error")?;
            message.extend_from_slice(&value);
            self.sender.send(message);
        }
        w.insert(idx, value);
        Ok(())
    }

    pub(crate) fn append_item(&self, value: Bytes) -> Result<(), String> {
        check_value_size(&self.name, value.len())?;

        let mut w = self.list.write();
        if self.connected.load(Ordering::Acquire) {
            let header =
                ServerHeader::ValueVec(self.id, self.type_id, VecHeader::Add, value.len() as u32);
            let mut message = serialize(&header).map_err(|_| "Failed to serialize header")?;
            message.extend_from_slice(&value);
            self.sender.send(message);
        }
        w.push(value);
        Ok(())
    }
}

impl SyncTrait for ValueList {
    fn sync(&self) -> Result<(), ()> {
        let r = self.list.read();
        self.send_all(self.serialize_all(&r).map_err(|_| ())?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{TRANSFER_CHUNK_SIZE, deserialize_value};

    /// Returns `(header kind, item count, payload bytes)` per sent message.
    fn sent(
        receiver: &mut crate::server_core::net::sender::MessageReceiver,
    ) -> Vec<(&'static str, u64, usize)> {
        let mut messages = Vec::new();
        while let Ok(Some((message, _))) = receiver.try_recv() {
            let message = message.into_bytes();
            let (header, used) = deserialize_value::<ServerHeader>(&message).unwrap();
            let ServerHeader::ValueVec(7, 3, header, size) = header else {
                panic!("not a list message");
            };
            assert_eq!(message.len() - used, size as usize);
            let (kind, count) = match header {
                VecHeader::All(count) => ("all", count),
                VecHeader::Begin(count) => ("begin", count),
                VecHeader::Chunk(count) => ("chunk", count),
                VecHeader::End(count) => ("end", count),
                _ => panic!("not a replacement"),
            };
            messages.push((kind, count, message.len()));
        }
        messages
    }

    #[test]
    fn unsigned_indices_stay_within_the_list() {
        for (index, len, item, insert) in [
            (0usize, 0, None, Some(0)),
            (0, 3, Some(0), Some(0)),
            (2, 3, Some(2), Some(2)),
            (3, 3, None, Some(3)),
            (4, 3, None, None),
            (usize::MAX, 3, None, None),
        ] {
            assert_eq!(index.item(len), item, "item {index} of {len}");
            assert_eq!(index.insert(len), insert, "insert {index} of {len}");
        }
    }

    #[test]
    fn signed_indices_resolve_like_python_lists() {
        for (index, len, item, insert) in [
            (0isize, 0, None, Some(0)),
            (1, 0, None, None),
            (-1, 0, None, None),
            (0, 3, Some(0), Some(0)),
            (2, 3, Some(2), Some(2)),
            (3, 3, None, Some(3)),
            (4, 3, None, None),
            (-1, 3, Some(2), Some(2)),
            (-3, 3, Some(0), Some(0)),
            (-4, 3, None, None),
            (isize::MIN, 3, None, None),
            (isize::MAX, 3, None, None),
        ] {
            assert_eq!(index.item(len), item, "item {index} of {len}");
            assert_eq!(index.insert(len), insert, "insert {index} of {len}");
        }
    }

    /// A negative index is resolved against the length the list has when the
    /// operation runs, and the client is sent the resolved position.
    #[test]
    fn a_signed_index_counts_from_the_current_end() {
        let (sender, mut receiver) = MessageSender::new();
        let list = ValueList::new(
            "root.list".into(),
            7,
            3,
            sender,
            Arc::new(AtomicBool::new(true)),
        );
        let item = |byte: u8| Bytes::from(vec![byte]);
        list.set(vec![item(1), item(2), item(3)]).unwrap();
        let _ = sent(&mut receiver);

        assert_eq!(list.get_item(-1isize).unwrap(), item(3));
        list.set_item_py(-1isize, item(4)).unwrap();
        list.insert_item(-1isize, item(5)).unwrap();
        assert_eq!(list.get(), [item(1), item(2), item(5), item(4)]);
        assert_eq!(list.remove_item(-4isize).unwrap(), item(1));
        assert_eq!(list.get(), [item(2), item(5), item(4)]);

        let mut positions = Vec::new();
        while let Ok(Some((message, _))) = receiver.try_recv() {
            let message = message.into_bytes();
            let (header, _) = deserialize_value::<ServerHeader>(&message).unwrap();
            let ServerHeader::ValueVec(7, 3, header, _) = header else {
                panic!("not a list message");
            };
            positions.push(match header {
                VecHeader::Set(index) => ("set", index),
                VecHeader::Insert(index) => ("insert", index),
                VecHeader::Remove(index) => ("remove", index),
                _ => panic!("not an item message"),
            });
        }
        assert_eq!(positions, [("set", 2), ("insert", 2), ("remove", 0)]);

        for index in [-4isize, 3] {
            assert!(list.get_item(index).is_err());
            assert!(list.set_item_py(index, item(9)).is_err());
            assert!(list.remove_item(index).is_err());
        }
        for index in [-4isize, 4] {
            assert!(list.insert_item(index, item(9)).is_err());
        }
        assert_eq!(list.get(), [item(2), item(5), item(4)]);
    }

    #[test]
    fn insert_sends_only_the_new_item() {
        let (sender, mut receiver) = MessageSender::new();
        let list = ValueList::new(
            "root.list".into(),
            7,
            3,
            sender,
            Arc::new(AtomicBool::new(true)),
        );
        let item = |byte: u8| Bytes::from(vec![byte]);
        list.set(vec![item(1), item(3)]).unwrap();
        let _ = sent(&mut receiver);

        list.insert_item(1usize, item(2)).unwrap();
        list.insert_item(3usize, item(4)).unwrap();
        assert!(list.insert_item(5usize, item(9)).is_err());
        assert_eq!(list.get(), [item(1), item(2), item(3), item(4)]);

        let mut inserts = Vec::new();
        while let Ok(Some((message, _))) = receiver.try_recv() {
            let message = message.into_bytes();
            let (header, used) = deserialize_value::<ServerHeader>(&message).unwrap();
            let ServerHeader::ValueVec(7, 3, VecHeader::Insert(index), 1) = header else {
                panic!("not an insert of one byte");
            };
            inserts.push((index, message[used]));
        }
        assert_eq!(inserts, [(1, 2), (3, 4)]);
    }

    #[test]
    fn a_large_replacement_is_split_into_chunks() {
        let (sender, mut receiver) = MessageSender::new();
        let list = ValueList::new(
            "root.list".into(),
            7,
            3,
            sender,
            Arc::new(AtomicBool::new(true)),
        );
        let kinds = |messages: &[(&'static str, u64, usize)]| {
            messages
                .iter()
                .map(|(kind, count, _)| (*kind, *count))
                .collect::<Vec<_>>()
        };

        list.set(vec![Bytes::from(vec![1u8; 4]); 2]).unwrap();
        assert_eq!(kinds(&sent(&mut receiver)), [("all", 2)]);

        list.set(vec![Bytes::from(vec![0u8; TRANSFER_CHUNK_SIZE / 10]); 25])
            .unwrap();
        let messages = sent(&mut receiver);
        assert_eq!(kinds(&messages), [("begin", 10), ("chunk", 10), ("end", 5)]);
        // Headers are a few bytes on top of the chunk size.
        assert!(
            messages
                .iter()
                .all(|(.., len)| *len < TRANSFER_CHUNK_SIZE + 64)
        );
    }
}
