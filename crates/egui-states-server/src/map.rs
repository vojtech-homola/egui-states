use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::collections::MapHeader;
use egui_states_core::serialization::{ServerHeader, serialize_value_vec};

use crate::sender::MessageSender;
use crate::server::{EnableTrait, SyncTrait};

pub(crate) struct ValueMap {
    id: u64,
    map: RwLock<HashMap<Bytes, Bytes>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    enabled: AtomicBool,
}

impl ValueMap {
    pub(crate) fn new(id: u64, sender: MessageSender, connected: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            id,
            map: RwLock::new(HashMap::new()),
            sender,
            connected,
            enabled: AtomicBool::new(false),
        })
    }

    fn serialize_all(&self, map: &HashMap<Bytes, Bytes>, update: bool) -> Bytes {
        let mut data = Vec::new();
        let len = map.len() as u64;
        let header = ServerHeader::Map(self.id, update, MapHeader::All);
        serialize_value_vec(&header, &mut data);
        serialize_value_vec(&len, &mut data);
        map.iter().for_each(|(k, v)| {
            data.extend_from_slice(&k);
            data.extend_from_slice(&v);
        });
        Bytes::from_owner(data)
    }

    pub(crate) fn set(&self, map: HashMap<Bytes, Bytes>, update: bool) {
        let mut w = self.map.write();

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let data = self.serialize_all(&map, update);
            self.sender.send(data);
        }

        *w = map;
    }

    pub(crate) fn get(&self) -> HashMap<Bytes, Bytes> {
        self.map.read().clone()
    }

    pub(crate) fn set_item(&self, key: Bytes, value: Bytes, update: bool) {
        let mut w = self.map.write();

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::Map(self.id, update, MapHeader::Set);
            let mut data = Vec::new();
            serialize_value_vec(&header, &mut data);
            data.extend_from_slice(&key);
            data.extend_from_slice(&value);
            self.sender.send(Bytes::from_owner(data));
        }

        match w.get_mut(&key) {
            Some(v) => *v = value,
            None => {
                w.insert(key, value);
            }
        }
    }

    pub(crate) fn get_item(&self, key: &Bytes) -> Option<Bytes> {
        match self.map.read().get(key) {
            Some(v) => Some(v.clone()),
            None => None,
        }
    }

    pub(crate) fn remove_item(&self, key: &Bytes, update: bool) -> Option<Bytes> {
        let mut w = self.map.write();
        let old = w.remove(key)?;

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::Map(self.id, update, MapHeader::Remove);
            let mut data = Vec::new();
            serialize_value_vec(&header, &mut data);
            data.extend_from_slice(&key);
            self.sender.send(Bytes::from_owner(data));
        }

        drop(w);
        Some(old)
    }

    pub(crate) fn len(&self) -> usize {
        self.map.read().len()
    }
}

impl SyncTrait for ValueMap {
    fn sync(&self) {}
}

impl EnableTrait for ValueMap {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}
