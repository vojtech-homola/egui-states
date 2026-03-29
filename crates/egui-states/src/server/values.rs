use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;

use crate::serialization::ServerHeader;
use crate::server::sender::MessageSender;
use crate::server::server::{Acknowledge, SyncTrait};
use crate::server::signals::SignalsManager;

// Value --------------------------------------------------
pub(crate) struct Value {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    value: RwLock<(Bytes, usize)>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    signals: SignalsManager,
}

impl Value {
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        value: Bytes,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        signals: SignalsManager,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            value: RwLock::new((value, 0)),
            sender,
            connected,
            signals,
        })
    }

    pub(crate) fn update_value(&self, type_id: u32, signal: bool, value: Bytes) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for Value: {}", self.name));
        }

        let mut w = self.value.write();
        if w.1 == 0 {
            if signal {
                self.signals.set(self.id, value.clone());
            }

            w.0 = value;
        }

        Ok(())
    }

    #[inline]
    pub(crate) fn get(&self) -> Bytes {
        self.value.read().0.clone()
    }

    pub(crate) fn set(&self, value: Bytes, set_signals: bool, update: bool) -> Result<(), ()> {
        if self.connected.load(Ordering::Relaxed) {
            let mut w = self.value.write();
            let message = ServerHeader::serialize_value(self.id, self.type_id, update, &value)?;

            w.0 = value.clone();
            w.1 += 1;
            self.sender.send(message);

            if set_signals {
                self.signals.set(self.id, value);
            }
        } else {
            let mut w = self.value.write();
            w.0 = value.clone();
            if set_signals {
                self.signals.set(self.id, value);
            }
        }
        Ok(())
    }
}

impl Acknowledge for Value {
    fn acknowledge(&self) {
        let mut w = self.value.write();
        if w.1 > 0 {
            w.1 -= 1;
        }
    }
}

impl SyncTrait for Value {
    fn sync(&self) -> Result<(), ()> {
        let mut w = self.value.write();
        w.1 = 1;
        let data = ServerHeader::serialize_value(self.id, self.type_id, false, &w.0)?;
        drop(w);

        self.sender.send(data);
        Ok(())
    }
}

// ValueStatic --------------------------------------------
pub(crate) struct ValueStatic {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    value: RwLock<Bytes>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueStatic {
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        value: Bytes,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            value: RwLock::new(value),
            sender,
            connected,
        })
    }

    pub(crate) fn set(&self, value: Bytes, update: bool) -> Result<(), ()> {
        if self.connected.load(Ordering::Relaxed) {
            let mut w = self.value.write();
            let message = ServerHeader::serialize_static(self.id, self.type_id, update, &value)?;

            *w = value;
            self.sender.send(message);
        } else {
            let mut w = self.value.write();
            *w = value;
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn get(&self) -> Bytes {
        self.value.read().clone()
    }
}

impl SyncTrait for ValueStatic {
    fn sync(&self) -> Result<(), ()> {
        let w = self.value.read();
        let data = ServerHeader::serialize_static(self.id, self.type_id, false, &w)?;
        drop(w);

        self.sender.send(data);
        Ok(())
    }
}

// Signals --------------------------------------------
pub(crate) struct Signal {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    signals: SignalsManager,
}

impl Signal {
    pub(crate) fn new(name: String, id: u64, type_id: u32, signals: SignalsManager) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            signals,
        })
    }

    pub(crate) fn set(&self, value: Bytes) {
        self.signals.set(self.id, value);
    }

    pub(crate) fn update_signal(&self, type_id: u32, value: Bytes) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for Signal: {}", self.name));
        }

        self.signals.set(self.id, value);
        Ok(())
    }
}
