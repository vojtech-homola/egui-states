use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;

use crate::serialization::ServerHeader;
use crate::server::sender::MessageSender;
use crate::server::server::{Acknowledge, SyncTrait};
use crate::server::{event::Event, signals::SignalsManager};

pub(crate) struct DataHolder {
    pub data: *const u8,
    pub size: usize,
}

// Data --------------------------------------------------
pub(crate) struct Data {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    value: RwLock<(Bytes, Vec<u8>)>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl Data {
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        header: Bytes,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            value: RwLock::new((header, Vec::new())),
            sender,
            connected,
            event: Event::new(),
        })
    }

    pub(crate) fn set(&self, header: Bytes, data: DataHolder) -> Result<(), String> {
        let mut w = self.value.write();
        if w.0.is_empty() {
            w.0 = header;
        }
        w.1 = data;
        
        Ok(())
    }

    #[inline]
    pub(crate) fn get(&self) -> (Bytes, Vec<u8>) {
        self.value.read().clone()
    }
}


impl Acknowledge for Data {
    fn acknowledge(&self) {
        self.event.set();
    }
}