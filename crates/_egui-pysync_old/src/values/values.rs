use postcard;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use crate::transport::{WriteMessage, serialize};

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

pub(crate) trait UpdateValueClient: Send + Sync {
    fn update_value(&self, data: &[u8]) -> Result<(), String>;
}

// Value --------------------------------------------
pub struct Value<T> {
    id: u32,
    value: RwLock<T>,
    channel: Sender<WriteMessage>,
}

impl<T> Value<T>
where
    T: Serialize + Clone,
{
    pub(crate) fn new(id: u32, value: T, channel: Sender<WriteMessage>) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            channel,
        })
    }

    pub fn get(&self) -> T {
        self.value.read().unwrap().clone()
    }

    pub fn set(&self, value: T, signal: bool) {
        let message = WriteMessage::Value(self.id, signal, serialize(&value));
        let mut w = self.value.write().unwrap();
        self.channel.send(message).unwrap();
        *w = value;
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateValueClient for Value<T> {
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = postcard::from_bytes(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        let mut w = self.value.write().unwrap();
        *w = value;
        self.channel.send(WriteMessage::ack(self.id)).unwrap();
        Ok(())
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

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateValueClient for ValueStatic<T> {
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = postcard::from_bytes(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        *self.value.write().unwrap() = value;
        Ok(())
    }
}

// Signal --------------------------------------------
pub struct Signal<T> {
    id: u32,
    channel: Sender<WriteMessage>,
    phantom: PhantomData<T>,
}

impl<T: Serialize + Clone> Signal<T> {
    pub(crate) fn new(id: u32, channel: Sender<WriteMessage>) -> Arc<Self> {
        Arc::new(Self {
            id,
            channel,
            phantom: PhantomData,
        })
    }

    pub fn set(&self, value: impl Into<T>) {
        let message = serialize(&value.into());
        let message = WriteMessage::Signal(self.id, message);
        self.channel.send(message).unwrap();
    }
}
