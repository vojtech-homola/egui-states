use parking_lot::RwLock;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

use egui_states_core_2::serialization::{TYPE_STATIC, TYPE_VALUE, deserialize, serialize_vec};
use egui_states_core_2::values::ObjectType;

use crate::sender::MessageSender;
use crate::server::{Acknowledge, SyncTrait};
use crate::signals::ChangedValues;

pub(crate) trait UpdateValue: Send + Sync {
    fn update_value(&self, type_id: u64, signal: bool, value: Bytes) -> Result<(), String>;
}

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
}

impl UpdateValue for Value {
    fn update_value(&self, type_id: u64, signal: bool, value: Bytes) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!(
                "Type ID mismatch for value {}: expected {}, got {}",
                self.id, self.type_id, type_id
            ));
        }

        let mut w = self.value.write();
        if w.1 == 0 {
            w.0 = value.clone();
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
    fn sync(&self) {
        let mut w = self.value.write();
        w.1 = 1;
        let data = serialize_vec(self.id, (false, &w.0), TYPE_VALUE);
        drop(w);

        self.sender.send(Bytes::from_owner(data));
    }
}
