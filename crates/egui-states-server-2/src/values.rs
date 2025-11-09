use parking_lot::RwLock;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core_2::serialization::{TYPE_STATIC, TYPE_VALUE, deserialize, serialize_vec};
use egui_states_core_2::value_object::Object;

use crate::sender::MessageSender;
use crate::server::{Acknowledge, SyncTrait};
use crate::signals::ChangedValues;

pub(crate) trait UpdateValue: Send + Sync {
    fn update_value(&self, obj: Object) -> Result<(), String>;
}

pub(crate) struct Value {
    id: u64,
    value: RwLock<(Object, usize)>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    signals: ChangedValues,
}

impl Value {
    pub(crate) fn new(
        id: u64,
        value: Object,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        signals: ChangedValues,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new((value, 0)),
            sender,
            connected,
            signals,
        })
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

        self.sender.send(Bytes::from(data));
    }
}
