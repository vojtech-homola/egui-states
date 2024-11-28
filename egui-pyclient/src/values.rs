use std::marker::PhantomData;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use egui_pysync::transport::WriteMessage;
use egui_pysync::values::{ReadValue, WriteValue};

pub(crate) trait ValueUpdate: Send + Sync {
    fn update_value(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String>;
}

pub struct Diff<'a, T> {
    pub v: T,
    original: T,
    value: &'a Value<T>,
}

impl<'a, T: WriteValue + Clone + PartialEq> Diff<'a, T> {
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

pub struct DiffEnum<'a, T> {
    pub v: T,
    original: T,
    value: &'a Value<T>,
}

impl<'a, T: WriteValue + Clone + PartialEq> DiffEnum<'a, T> {
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
    channel: Sender<WriteMessage>,
}

impl<T> Value<T>
where
    T: WriteValue + Clone,
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
        let message = WriteMessage::Value(self.id, signal, value.clone().into_message());
        let mut w = self.value.write().unwrap();
        *w = value;
        self.channel.send(message).unwrap();
    }
}

impl<T: ReadValue> ValueUpdate for Value<T> {
    fn update_value(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String> {
        let value = T::read_message(head, data)
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

impl<T: ReadValue> ValueUpdate for ValueStatic<T> {
    fn update_value(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String> {
        let value = T::read_message(head, data)
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

impl<T: WriteValue + Clone> Signal<T> {
    pub(crate) fn new(id: u32, channel: Sender<WriteMessage>) -> Arc<Self> {
        Arc::new(Self {
            id,
            channel,
            phantom: PhantomData,
        })
    }

    pub fn set(&self, value: T) {
        let message = value.into_message();
        let message = WriteMessage::Signal(self.id, message);
        self.channel.send(message).unwrap();
    }
}
