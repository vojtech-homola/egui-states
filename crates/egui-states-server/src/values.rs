use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;

use egui_states_core::serialization::{ServerHeader, ser_server_value};

use crate::sender::MessageSender;
use crate::server::{Acknowledge, EnableTrait, SyncTrait};
use crate::signals::SignalsManager;

// pub(crate) trait UpdateValue: Send + Sync {
//     fn update_value(&self, signal: bool, value: Bytes) -> Result<(), String>;
// }

// pub(crate) trait GetValue: SetValue {
//     fn get_value(&self) -> Bytes;
//     fn get_type(&self) -> ObjectType;
// }

// pub(crate) trait SetValue: Send + Sync {
//     fn set_value(&self, value: Bytes);
// }

// Value --------------------------------------------------
pub(crate) struct Value {
    id: u64,
    value: RwLock<(Bytes, usize)>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    enabled: AtomicBool,
    signals: SignalsManager,
}

impl Value {
    pub(crate) fn new(
        id: u64,
        value: Bytes,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        signals: SignalsManager,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new((value, 0)),
            sender,
            connected,
            enabled: AtomicBool::new(false),
            signals,
        })
    }
    // }

    // impl UpdateValue for Value {
    pub(crate) fn update_value(&self, signal: bool, value: Bytes) -> Result<(), String> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Err(format!("Value {} is not enabled", self.id));
        }

        let mut w = self.value.write();
        if w.1 == 0 {
            w.0 = value.clone();
        }

        if signal {
            self.signals.set(self.id, value);
        }

        Ok(())
    }

    #[inline]
    pub(crate) fn get(&self) -> Bytes {
        self.value.read().0.clone()
    }

    pub(crate) fn set(&self, value: Bytes, set_signals: bool, update: bool) {
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let mut w = self.value.write();

            let header = ServerHeader::Value(self.id, update);
            let message = header.serialize_to_bytes_data(Some(value.clone()));

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
    fn sync(&self) {
        let mut w = self.value.write();
        w.1 = 1;
        let header = ServerHeader::Value(self.id, false);
        let data = ser_server_value(header, &w.0);
        drop(w);

        self.sender.send(data);
    }
}

impl EnableTrait for Value {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}

// ValueStatic --------------------------------------------
pub(crate) struct ValueStatic {
    id: u64,
    value: RwLock<Bytes>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    enabled: AtomicBool,
}

impl ValueStatic {
    pub(crate) fn new(
        id: u64,
        value: Bytes,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            sender,
            connected,
            enabled: AtomicBool::new(false),
        })
    }

    pub(crate) fn set(&self, value: Bytes, update: bool) {
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let mut w = self.value.write();

            let header = ServerHeader::Static(self.id, update);
            let message = header.serialize_to_bytes_data(Some(value.clone()));

            *w = value;
            self.sender.send(message);
        } else {
            let mut w = self.value.write();
            *w = value;
        }
    }

    #[inline]
    pub(crate) fn get(&self) -> Bytes {
        self.value.read().clone()
    }
}

impl SyncTrait for ValueStatic {
    fn sync(&self) {
        if self.enabled.load(Ordering::Relaxed) {
            let w = self.value.read();
            let header = ServerHeader::Static(self.id, false);
            let data = ser_server_value(header, &w);
            drop(w);

            self.sender.send(data);
        }
    }
}

impl EnableTrait for ValueStatic {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}

// Signals --------------------------------------------
pub(crate) struct Signal {
    id: u64,
    signals: SignalsManager,
    enabled: AtomicBool,
}

impl Signal {
    pub(crate) fn new(id: u64, signals: SignalsManager) -> Arc<Self> {
        Arc::new(Self {
            id,
            signals,
            enabled: AtomicBool::new(false),
        })
    }

    pub(crate) fn set(&self, value: Bytes) {
        self.signals.set(self.id, value);
    }

    pub(crate) fn update_signal(&self, value: Bytes) -> Result<(), String> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Err(format!("Signal {} is not enabled", self.id));
        }
        self.signals.set(self.id, value);
        Ok(())
    }
}

impl EnableTrait for Signal {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}
