use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

use crate::collections::{MapHeader, split_collection};
use crate::serialization::{ServerHeader, check_value_size, serialize};
use crate::server_core::sender::{MessageSender, SenderData};
use crate::server_core::server::SyncTrait;

pub(crate) struct ValueMap {
    // #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    map: RwLock<HashMap<Bytes, Bytes>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueMap {
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
            map: RwLock::new(HashMap::new()),
            sender,
            connected,
        })
    }

    /// Serializes a whole-map replacement, split into frame-sized messages.
    /// Only the last message requests a UI update.
    fn serialize_all(
        &self,
        map: &HashMap<Bytes, Bytes>,
        update: bool,
    ) -> Result<Vec<SenderData>, &'static str> {
        let chunks = split_collection(map.iter().map(|(k, v)| k.len() + v.len()));
        let last = chunks.len() - 1;
        let mut items = map.iter();
        let mut messages = Vec::with_capacity(chunks.len());
        for (i, (part, count, size)) in chunks.into_iter().enumerate() {
            let header = ServerHeader::ValueMap(
                self.id,
                self.type_id,
                update && i == last,
                MapHeader::part(part, count),
                size as u32,
            );
            let mut data = serialize(&header).map_err(|_| "Failed to serialize header")?;
            items.by_ref().take(count as usize).for_each(|(k, v)| {
                data.extend_from_slice(k);
                data.extend_from_slice(v);
            });
            messages.push(data);
        }
        Ok(messages)
    }

    fn send_all(&self, messages: Vec<SenderData>) {
        for message in messages {
            self.sender.send(message);
        }
    }

    pub(crate) fn set(&self, map: HashMap<Bytes, Bytes>, update: bool) -> Result<(), String> {
        // the limit applies to single items, not to the whole map
        for (key, value) in map.iter() {
            check_value_size(&self.name, key.len() + value.len())?;
        }

        let mut w = self.map.write();

        if self.connected.load(Ordering::Acquire) {
            self.send_all(self.serialize_all(&map, update)?);
        }

        *w = map;
        Ok(())
    }

    pub(crate) fn get(&self) -> HashMap<Bytes, Bytes> {
        self.map.read().clone()
    }

    pub(crate) fn set_item(&self, key: Bytes, value: Bytes, update: bool) -> Result<(), String> {
        check_value_size(&self.name, key.len() + value.len())?;

        let mut w = self.map.write();

        if self.connected.load(Ordering::Acquire) {
            let header = ServerHeader::ValueMap(
                self.id,
                self.type_id,
                update,
                MapHeader::Set,
                (key.len() + value.len()) as u32,
            );
            let mut data = serialize(&header).map_err(|_| "Failed to serialize header")?;
            data.extend_from_slice(&key);
            data.extend_from_slice(&value);
            self.sender.send(data);
        }

        match w.get_mut(&key) {
            Some(v) => *v = value,
            None => {
                w.insert(key, value);
            }
        }
        Ok(())
    }

    pub(crate) fn get_item(&self, key: &Bytes) -> Option<Bytes> {
        match self.map.read().get(key) {
            Some(v) => Some(v.clone()),
            None => None,
        }
    }

    pub(crate) fn remove_item(&self, key: &Bytes, update: bool) -> Result<Option<Bytes>, ()> {
        let mut w = self.map.write();
        let old = match w.remove(key) {
            Some(v) => v,
            None => return Ok(None),
        };

        if self.connected.load(Ordering::Acquire) {
            let header = ServerHeader::ValueMap(
                self.id,
                self.type_id,
                update,
                MapHeader::Remove,
                key.len() as u32,
            );
            let mut data = serialize(&header)?;
            data.extend_from_slice(&key);
            self.sender.send(data);
        }

        drop(w);
        Ok(Some(old))
    }

    pub(crate) fn len(&self) -> usize {
        self.map.read().len()
    }
}

impl SyncTrait for ValueMap {
    fn sync(&self) -> Result<(), ()> {
        let r = self.map.read();
        self.send_all(self.serialize_all(&r, false).map_err(|_| ())?);
        Ok(())
    }
}
