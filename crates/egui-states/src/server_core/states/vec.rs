use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;

use crate::protocol::collections::{VecHeader, serialize_replacement};
use crate::protocol::{ServerHeader, check_value_size, serialize};
use crate::server_core::net::sender::{MessageSender, SenderData};
use crate::server_core::server::SyncTrait;

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
    /// Only the last message requests a UI update.
    fn serialize_all(&self, vec: &[Bytes], update: bool) -> Result<Vec<SenderData>, &'static str> {
        serialize_replacement(
            vec.iter(),
            |item| item.len(),
            |message, item| message.extend_from_slice(item),
            |part, count, size, update| {
                ServerHeader::ValueVec(
                    self.id,
                    self.type_id,
                    update,
                    VecHeader::part(part, count),
                    size,
                )
            },
            update,
        )
    }

    fn send_all(&self, messages: Vec<SenderData>) {
        for message in messages {
            self.sender.send(message);
        }
    }

    pub(crate) fn set(&self, list: Vec<Bytes>, update: bool) -> Result<(), String> {
        // the limit applies to single items, not to the whole list
        for value in list.iter() {
            check_value_size(&self.name, value.len())?;
        }

        let mut w = self.list.write();

        if self.connected.load(Ordering::Acquire) {
            self.send_all(self.serialize_all(&list, update)?);
        }

        *w = list;
        Ok(())
    }

    pub(crate) fn get(&self) -> Vec<Bytes> {
        self.list.read().clone()
    }

    pub(crate) fn set_item_py(&self, idx: usize, value: Bytes, update: bool) -> Result<(), String> {
        check_value_size(&self.name, value.len())?;

        let mut w = self.list.write();
        if idx >= w.len() {
            return Err("Index out of bounds".to_string());
        }

        if self.connected.load(Ordering::Acquire) {
            let header = ServerHeader::ValueVec(
                self.id,
                self.type_id,
                update,
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

    pub(crate) fn get_item(&self, idx: usize) -> Result<Bytes, &'static str> {
        let r = self.list.read();
        if idx >= r.len() {
            return Err("Index out of bounds");
        }
        Ok(r[idx].clone())
    }

    pub(crate) fn len(&self) -> usize {
        self.list.read().len()
    }

    pub(crate) fn remove_item(&self, idx: usize, update: bool) -> Result<Bytes, &'static str> {
        let mut w = self.list.write();
        if idx >= w.len() {
            return Err("Index out of bounds");
        }
        let value = w.remove(idx);

        if self.connected.load(Ordering::Acquire) {
            let header = ServerHeader::ValueVec(
                self.id,
                self.type_id,
                update,
                VecHeader::Remove(idx as u64),
                0,
            );
            let message = serialize(&header).map_err(|_| "Serialization error")?;
            self.sender.send(message);
        }

        Ok(value)
    }

    pub(crate) fn append_item(&self, value: Bytes, update: bool) -> Result<(), String> {
        check_value_size(&self.name, value.len())?;

        let mut w = self.list.write();
        if self.connected.load(Ordering::Acquire) {
            let header = ServerHeader::ValueVec(
                self.id,
                self.type_id,
                update,
                VecHeader::Add,
                value.len() as u32,
            );
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
        self.send_all(self.serialize_all(&r, false).map_err(|_| ())?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{TRANSFER_CHUNK_SIZE, deserialize_value};

    /// Returns `(header kind, item count, update, payload bytes)` per sent message.
    fn sent(
        receiver: &mut crate::server_core::net::sender::MessageReceiver,
    ) -> Vec<(&'static str, u64, bool, usize)> {
        let mut messages = Vec::new();
        while let Ok(Some((message, _))) = receiver.try_recv() {
            let message = message.into_bytes();
            let (header, used) = deserialize_value::<ServerHeader>(&message).unwrap();
            let ServerHeader::ValueVec(7, 3, update, header, size) = header else {
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
            messages.push((kind, count, update, message.len()));
        }
        messages
    }

    #[test]
    fn a_large_replacement_is_split_and_only_its_end_requests_an_update() {
        let (sender, mut receiver) = MessageSender::new();
        let list = ValueList::new(
            "root.list".into(),
            7,
            3,
            sender,
            Arc::new(AtomicBool::new(true)),
        );
        let kinds = |messages: &[(&'static str, u64, bool, usize)]| {
            messages
                .iter()
                .map(|(kind, count, update, _)| (*kind, *count, *update))
                .collect::<Vec<_>>()
        };

        list.set(vec![Bytes::from(vec![1u8; 4]); 2], true).unwrap();
        assert_eq!(kinds(&sent(&mut receiver)), [("all", 2, true)]);

        list.set(
            vec![Bytes::from(vec![0u8; TRANSFER_CHUNK_SIZE / 10]); 25],
            true,
        )
        .unwrap();
        let messages = sent(&mut receiver);
        assert_eq!(
            kinds(&messages),
            [("begin", 10, false), ("chunk", 10, false), ("end", 5, true)]
        );
        // Headers are a few bytes on top of the chunk size.
        assert!(
            messages
                .iter()
                .all(|(.., len)| *len < TRANSFER_CHUNK_SIZE + 64)
        );
    }
}
