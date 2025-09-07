use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use egui_states_core::controls::ControlMessage;
use egui_states_core::serialization::{TYPE_SIGNAL, TYPE_VALUE, deserialize, serialize};

use crate::UpdateValue;
use crate::sender::MessageSender;

pub struct Diff<'a, T> {
    pub v: T,
    original: T,
    value: &'a Value<T>,
}

impl<'a, T: Serialize + Clone + PartialEq> Diff<'a, T> {
    pub fn new(value: &'a Value<T>) -> Self {
        let v = value.get();
        Self {
            v: v.clone(),
            original: v,
            value,
        }
    }

    #[inline]
    pub fn set(self, signal: bool) {
        if self.v != self.original {
            self.value.set(self.v, signal);
        }
    }
}

// Value --------------------------------------------
pub struct Value<T> {
    id: u32,
    value: RwLock<T>,
    sender: MessageSender,
}

impl<T> Value<T>
where
    T: Serialize + Clone,
{
    pub(crate) fn new(id: u32, value: T, sender: MessageSender) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            sender,
        })
    }

    pub fn get(&self) -> T {
        self.value.read().unwrap().clone()
    }

    pub fn set(&self, value: T, signal: bool) {
        let data = serialize(self.id, (signal, &value), TYPE_VALUE);
        let mut w = self.value.write().unwrap();
        self.sender.send(data);
        *w = value;
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateValue for Value<T> {
    fn update_value(&self, data: &[u8]) -> Result<bool, String> {
        let (update, value) = deserialize::<(bool, T)>(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        let mut w = self.value.write().unwrap();
        *w = value;
        self.sender.send(ControlMessage::ack(self.id));
        Ok(update)
    }
}

// StaticValue --------------------------------------------
pub struct ValueStatic<T> {
    id: u32,
    value: RwLock<T>,
}

impl<T: Clone> ValueStatic<T> {
    pub(crate) fn new(id: u32, value: T) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
        })
    }

    pub fn get(&self) -> T {
        self.value.read().unwrap().clone()
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateValue for ValueStatic<T> {
    fn update_value(&self, data: &[u8]) -> Result<bool, String> {
        let (update, value) = deserialize::<(bool, T)>(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;
        *self.value.write().unwrap() = value;
        Ok(update)
    }
}

// Signal --------------------------------------------
pub struct Signal<T> {
    id: u32,
    sender: MessageSender,
    phantom: PhantomData<T>,
}

impl<T: Serialize + Clone> Signal<T> {
    pub(crate) fn new(id: u32, sender: MessageSender) -> Arc<Self> {
        Arc::new(Self {
            id,
            sender,
            phantom: PhantomData,
        })
    }

    pub fn set(&self, value: impl Into<T>) {
        let message = serialize(self.id, &value.into(), TYPE_SIGNAL);
        self.sender.send(message);
    }
}
