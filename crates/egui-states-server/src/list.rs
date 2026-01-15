use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::collections::ListHeader;
use egui_states_core::serialization::{FastVec, ServerHeader, serialize};

use crate::sender::{MessageSender, SenderData};
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

    fn serialize_all(&self, vec: &Vec<Bytes>, update: bool) -> Result<SenderData, ()> {
        let len = vec.len() as u64;
        let len_data: FastVec<10> = serialize(&len)?;
        let mut size = 0;
        vec.iter().for_each(|b| {
            size += b.len();
        });
        let all_size = (size + len_data.len()) as u32;
        let header = ServerHeader::List(self.id, update, ListHeader::All, all_size);

        let mut data = serialize(&header)?;
        data.extend_from_data(&len_data);
        vec.iter().for_each(|b| {
            data.extend_from_slice(&b);
        });

        Ok(data)
    }

    pub(crate) fn set(&self, list: Vec<Bytes>, update: bool) -> Result<(), ()> {
        let mut w = self.list.write();

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let data = self.serialize_all(&list, update)?;
            self.sender.send(data);
        }

        *w = list;
        Ok(())
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
            let header = ServerHeader::List(
                self.id,
                update,
                ListHeader::Set(idx as u64),
                value.len() as u32,
            );
            let message = serialize(&header).map_err(|_| "Serialization error")?;
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
            let header = ServerHeader::List(self.id, update, ListHeader::Remove(idx as u64), 0);
            let message = serialize(&header).map_err(|_| "Serialization error")?;
            self.sender.send(message);
        }

        Ok(value)
    }

    pub(crate) fn append_item(&self, value: Bytes, update: bool) -> Result<(), ()> {
        let mut w = self.list.write();
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::List(self.id, update, ListHeader::Add, value.len() as u32);
            let message = serialize(&header)?;
            self.sender.send(message);
        }
        w.push(value);
        Ok(())
    }
}

impl SyncTrait for ValueList {
    fn sync(&self) -> Result<(), ()> {
        if self.enabled.load(Ordering::Relaxed) {
            let r = self.list.read();
            let data = self.serialize_all(&r, false)?;
            self.sender.send(data);
        }
        Ok(())
    }
}

impl EnableTrait for ValueList {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}
