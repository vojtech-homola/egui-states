use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::collections::ListHeader;
use egui_states_core::serialization::{ServerHeader, serialize_value_vec};

use crate::sender::MessageSender;
use crate::server::{EnableTrait, SyncTrait};

pub(crate) struct ValueList {
    id: u64,
    list: RwLock<Vec<Bytes>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    enabled: AtomicBool,
}

impl ValueList {
    pub(crate) fn new(id: u64, sender: MessageSender, connected: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            id,
            list: RwLock::new(Vec::new()),
            sender,
            connected,
            enabled: AtomicBool::new(false),
        })
    }

    fn serialize_all(&self, vec: &Vec<Bytes>, update: bool) -> Bytes {
        let mut data = Vec::new();
        let len = vec.len() as u64;
        let header = ServerHeader::List(self.id, update, ListHeader::All);
        serialize_value_vec(&header, &mut data);
        serialize_value_vec(&len, &mut data);
        vec.iter().for_each(|b| {
            data.extend_from_slice(&b);
        });
        Bytes::from_owner(data)
    }

    pub(crate) fn set(&self, list: Vec<Bytes>, update: bool) {
        let mut w = self.list.write();

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let data = self.serialize_all(&list, update);
            self.sender.send(data);
        }

        *w = list;
    }

    pub(crate) fn get(&self) -> Vec<Bytes> {
        self.list.read().clone()
    }

    pub(crate) fn set_item_py(
        &self,
        idx: usize,
        value: Bytes,
        update: bool,
    ) -> Result<(), &'static str> {
        let mut w = self.list.write();
        if idx >= w.len() {
            return Err("Index out of bounds");
        }

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::List(self.id, update, ListHeader::Set(idx as u64));
            let message = header.serialize_to_bytes_data(Some(value.clone()));
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

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::List(self.id, update, ListHeader::Remove(idx as u64));
            let message = header.serialize_to_bytes();
            self.sender.send(message);
        }

        Ok(value)
    }

    pub(crate) fn append_item(&self, value: Bytes, update: bool) {
        let mut w = self.list.write();
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::List(self.id, update, ListHeader::Add);
            let message = header.serialize_to_bytes_data(Some(value.clone()));
            self.sender.send(message);
        }
        w.push(value);
    }
}

impl SyncTrait for ValueList {
    fn sync(&self) {
        if self.enabled.load(Ordering::Relaxed) {
            let r = self.list.read();
            let data = self.serialize_all(&r, false);
            self.sender.send(data);
        }
    }
}

impl EnableTrait for ValueList {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}
