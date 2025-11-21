use parking_lot::RwLock;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

use egui_states_core::serialization::{ServerHeader, deserialize, ser_server_value};
use egui_states_core::values::ObjectType;

use crate::sender::MessageSender;
use crate::server::{Acknowledge, SyncTrait};
use crate::signals::ChangedValues;

// pub(crate) trait UpdateValue: Send + Sync {
//     fn update_value(&self, signal: bool, value: Bytes) -> Result<(), String>;
// }

pub(crate) trait GetValue: SetValue {
    fn get_value(&self) -> Bytes;
    fn get_type(&self) -> ObjectType;
}

pub(crate) trait SetValue: Send + Sync {
    fn set_value(&self, value: Bytes);
}

// Value --------------------------------------------------
pub(crate) struct Value {
    id: u64,
    value: RwLock<(Bytes, usize)>,
    value_type: ObjectType,
    type_id: u64,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl Value {
    pub(crate) fn new(
        id: u64,
        value: Bytes,
        value_type: ObjectType,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let type_id = value_type.get_hash();
        Arc::new(Self {
            id,
            value: RwLock::new((value, 0)),
            value_type,
            type_id,
            sender,
            connected,
        })
    }
    // }

    // impl UpdateValue for Value {
    pub(crate) fn update_value(&self, signal: bool, value: Bytes) -> Result<(), String> {
        let mut w = self.value.write();
        if w.1 == 0 {
            w.0 = value.clone();
        }

        Ok(())
    }
}

impl GetValue for Value {
    #[inline]
    fn get_value(&self) -> Bytes {
        self.value.read().0.clone()
    }

    #[inline]
    fn get_type(&self) -> ObjectType {
        self.value_type.clone()
    }
}

impl SetValue for Value {
    fn set_value(&self, value: Bytes) {
        let mut w = self.value.write();
        w.0 = value;
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

// ValueStatic --------------------------------------------
pub(crate) struct ValueStatic {
    id: u64,
    value: RwLock<Bytes>,
    value_type: ObjectType,
    type_id: u64,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueStatic {
    pub(crate) fn new(
        id: u64,
        value: Bytes,
        value_type: ObjectType,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let type_id = value_type.get_hash();
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            value_type,
            type_id,
            sender,
            connected,
        })
    }
}

// Signals --------------------------------------------
pub(crate) struct Signal {
    id: u64,
    value_type: ObjectType,
    type_id: u64,
}

impl Signal {
    pub(crate) fn new(id: u64, value: Bytes, value_type: ObjectType) -> Arc<Self> {
        let type_id = value_type.get_hash();
        Arc::new(Self {
            id,
            value_type,
            type_id,
        })
    }
// }

// impl UpdateValue for Signal {
    pub(crate) fn set_signal(&self, value: Bytes) -> Result<(), String> {
        Ok(())
    }
}
