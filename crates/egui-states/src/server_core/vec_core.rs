use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

use crate::collections::VecHeader;
use crate::serialization::{ServerHeader, check_value_size, serialize};
use crate::server_core::sender::{MessageSender, SenderData};
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

    fn serialize_all(&self, vec: &Vec<Bytes>, update: bool) -> Result<SenderData, &'static str> {
        let len = vec.len() as u64;
        let mut size = 0;
        vec.iter().for_each(|b| {
            size += b.len();
        });
        let header = ServerHeader::ValueVec(
            self.id,
            self.type_id,
            update,
            VecHeader::All(len),
            size as u32,
        );

        let mut data = serialize(&header).map_err(|_| "Failed to serialize header")?;
        vec.iter().for_each(|b| {
            data.extend_from_slice(&b);
        });

        Ok(data)
    }

    pub(crate) fn set(&self, list: Vec<Bytes>, update: bool) -> Result<(), String> {
        // the limit applies to single items, not to the whole list
        for value in list.iter() {
            check_value_size(&self.name, value.len())?;
        }

        let mut w = self.list.write();

        if self.connected.load(Ordering::Acquire) {
            let data = self.serialize_all(&list, update)?;
            self.sender.send(data);
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
        let data = self.serialize_all(&r, false).map_err(|_| ())?;
        self.sender.send(data);
        Ok(())
    }
}
