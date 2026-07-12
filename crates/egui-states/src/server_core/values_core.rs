use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;

use crate::event::Event;
use crate::serialization::ServerHeader;
use crate::server_core::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::signals::SignalsManager;

// Value --------------------------------------------------
pub(crate) struct ValueCore {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    value: RwLock<(Bytes, usize)>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    signals: SignalsManager,
}

impl ValueCore {
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

    pub(crate) fn update_value(
        &self,
        type_id: u32,
        signal: bool,
        value: Bytes,
    ) -> Result<(), String> {
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

impl Acknowledge for ValueCore {
    fn acknowledge(&self) {
        let mut w = self.value.write();
        if w.1 > 0 {
            w.1 -= 1;
        }
    }
}

impl SyncTrait for ValueCore {
    fn sync(&self) -> Result<(), ()> {
        let mut w = self.value.write();
        w.1 = 1;
        let data = ServerHeader::serialize_value(self.id, self.type_id, false, &w.0)?;
        drop(w);

        self.sender.send(data);
        Ok(())
    }
}

// ValueTake --------------------------------------------------
pub(crate) struct ValueTakeCore {
    // #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    event: Event,
    lock: RwLock<()>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueTakeCore {
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
            event: Event::new(),
            lock: RwLock::new(()),
            sender,
            connected,
        })
    }

    pub(crate) fn set(&self, value: Bytes, blocking: bool, update: bool) -> Result<(), ()> {
        if self.connected.load(Ordering::Relaxed) {
            let message =
                ServerHeader::serialize_value_take(self.id, self.type_id, blocking, update, &value)
                    .map_err(|_| ())?;

            let _guard = self.lock.write();

            match blocking {
                true => self.event.wait_clear(),
                false => self.event.wait(),
            }
            if !self.connected.load(Ordering::Relaxed) {
                return Ok(());
            }

            self.sender.send(message);
        }
        Ok(())
    }
}

impl Acknowledge for ValueTakeCore {
    fn acknowledge(&self) {
        self.event.set();
    }
}

impl SyncTrait for ValueTakeCore {
    fn sync(&self) -> Result<(), ()> {
        self.event.set();
        Ok(())
    }
}

// ValueStatic --------------------------------------------
pub(crate) struct ValueStaticCore {
    // #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    value: RwLock<Bytes>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueStaticCore {
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

impl SyncTrait for ValueStaticCore {
    fn sync(&self) -> Result<(), ()> {
        let w = self.value.read();
        let data = ServerHeader::serialize_static(self.id, self.type_id, false, &w)?;
        drop(w);

        self.sender.send(data);
        Ok(())
    }
}

// Signals --------------------------------------------
pub(crate) struct SignalCore {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    signals: SignalsManager,
}

impl SignalCore {
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
