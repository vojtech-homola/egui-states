use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;

use egui_states_core::serialization::{ClientHeader, deserialize, serialize_value_to_message};

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
    pub fn set(self) {
        if self.v != self.original {
            self.value.set(self.v);
        }
    }

    #[inline]
    pub fn set_signal(self) {
        if self.v != self.original {
            self.value.set_signal(self.v);
        }
    }
}

pub trait UpdateValue: Sync + Send {
    fn update_value(&self, data: &[u8]) -> Result<(), String>;
}

// Value --------------------------------------------
pub struct Value<T> {
    id: u64,
    value: RwLock<T>,
    sender: MessageSender,
}

impl<T> Value<T>
where
    T: Serialize + Clone,
{
    pub(crate) fn new(id: u64, value: T, sender: MessageSender) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            sender,
        })
    }

    pub fn get(&self) -> T {
        self.value.read().clone()
    }

    pub fn set(&self, value: T) {
        let data = serialize_value_to_message(&value);
        let header = ClientHeader::Value(self.id, false);
        let mut w = self.value.write();
        self.sender.send_data(header, data);
        *w = value;
    }

    pub fn set_signal(&self, value: T) {
        let data = serialize_value_to_message(&value);
        let header = ClientHeader::Value(self.id, true);
        let mut w = self.value.write();
        self.sender.send_data(header, data);
        *w = value;
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateValue for Value<T> {
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        let mut w = self.value.write();
        *w = value;
        self.sender.send(ClientHeader::ack(self.id));
        Ok(())
    }
}

// StaticValue --------------------------------------------
pub struct ValueStatic<T> {
    id: u64,
    value: RwLock<T>,
}

impl<T: Clone> ValueStatic<T> {
    pub(crate) fn new(id: u64, value: T) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
        })
    }

    pub fn get(&self) -> T {
        self.value.read().clone()
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateValue for ValueStatic<T> {
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;
        *self.value.write() = value;
        Ok(())
    }
}

// Signal --------------------------------------------
pub struct Signal<T> {
    id: u64,
    sender: MessageSender,
    phantom: PhantomData<T>,
}

impl<T: Serialize + Clone> Signal<T> {
    pub(crate) fn new(id: u64, sender: MessageSender) -> Arc<Self> {
        Arc::new(Self {
            id,
            sender,
            phantom: PhantomData,
        })
    }

    pub fn set(&self, value: impl Into<T>) {
        let message = serialize_value_to_message(&value.into());
        let header = ClientHeader::Signal(self.id);
        self.sender.send_data(header, message);
    }
}
