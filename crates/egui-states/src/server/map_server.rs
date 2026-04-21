use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

use crate::collections::MapHeader;
use crate::serialization::{ServerHeader, serialize};
use crate::server::sender::{MessageSender, SenderData};
use crate::server::server::SyncTrait;

pub(crate) struct ValueMap {
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

    fn serialize_all(&self, map: &HashMap<Bytes, Bytes>, update: bool) -> Result<SenderData, ()> {
        let len = map.len() as u64;
        let mut size = 0;
        map.iter().for_each(|(k, v)| {
            size += k.len();
            size += v.len();
        });
        let header = ServerHeader::ValueMap(
            self.id,
            self.type_id,
            update,
            MapHeader::All(len),
            size as u32,
        );

        let mut data = serialize(&header)?;
        map.iter().for_each(|(k, v)| {
            data.extend_from_slice(&k);
            data.extend_from_slice(&v);
        });

        Ok(data)
    }

    pub(crate) fn set(&self, map: HashMap<Bytes, Bytes>, update: bool) -> Result<(), ()> {
        let mut w = self.map.write();

        if self.connected.load(Ordering::Relaxed) {
            let data = self.serialize_all(&map, update)?;
            self.sender.send(data);
        }

        *w = map;
        Ok(())
    }

    pub(crate) fn get(&self) -> HashMap<Bytes, Bytes> {
        self.map.read().clone()
    }

    pub(crate) fn set_item(&self, key: Bytes, value: Bytes, update: bool) -> Result<(), ()> {
        let mut w = self.map.write();

        if self.connected.load(Ordering::Relaxed) {
            let header = ServerHeader::ValueMap(
                self.id,
                self.type_id,
                update,
                MapHeader::Set,
                (key.len() + value.len()) as u32,
            );
            let mut data = serialize(&header)?;
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

        if self.connected.load(Ordering::Relaxed) {
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
        let data = self.serialize_all(&r, false)?;
        self.sender.send(data);
        Ok(())
    }
}
