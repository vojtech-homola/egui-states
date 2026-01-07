use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;

use egui_states_core::serialization::{deserialize, to_message};

use crate::sender::{ChannelMessage, MessageSender};

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

pub enum QueueType {
    Single,
    Multiple,
}

pub trait GetQueueType: Sync + Send + 'static {
    fn queue_type() -> QueueType;
}

pub struct NoQueue;

impl GetQueueType for NoQueue {
    #[inline]
    fn queue_type() -> QueueType {
        QueueType::Single
    }
}

pub struct Queue;

impl GetQueueType for Queue {
    #[inline]
    fn queue_type() -> QueueType {
        QueueType::Multiple
    }
}

// Value --------------------------------------------
pub struct Value<T, Q: GetQueueType = NoQueue> {
    id: u64,
    value: RwLock<T>,
    sender: MessageSender,
    _phantom: PhantomData<Q>,
}

// impl<T, Q: GetQueueType> GetQueueType for Value<T, Q> {
//     fn queue_type() -> QueueType {
//         Q::queue_type()
//     }
// }

impl<T, Q: GetQueueType> Value<T, Q>
where
    T: Serialize + Clone,
{
    pub(crate) fn new(id: u64, value: T, sender: MessageSender) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            sender,
            _phantom: PhantomData,
        })
    }

    pub fn get(&self) -> T {
        self.value.read().clone()
    }

    pub fn read<R>(&self, mut f: impl FnMut(&T) -> R) -> R {
        let r = self.value.read();
        f(&r)
    }

    pub fn write<R>(&self, mut f: impl FnMut(&mut T) -> R) -> R {
        let mut w = self.value.write();

        let result = f(&mut w);

        let data = to_message(&*w);
        self.sender
            .send(ChannelMessage::Value(self.id, false, data));
        result
    }

    pub fn write_signal<R>(&self, mut f: impl FnMut(&mut T) -> R) -> R {
        let mut w = self.value.write();

        let result = f(&mut w);

        let data = to_message(&*w);
        self.sender.send(ChannelMessage::Value(self.id, true, data));
        result
    }

    pub fn set(&self, value: T) {
        let data = to_message(&value);

        let mut w = self.value.write();
        self.sender
            .send(ChannelMessage::Value(self.id, false, data));
        *w = value;
    }

    pub fn set_signal(&self, value: T) {
        let data = to_message(&value);

        let mut w = self.value.write();
        self.sender.send(ChannelMessage::Value(self.id, true, data));
        *w = value;
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync, Q: GetQueueType + Send + Sync> UpdateValue
    for Value<T, Q>
{
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        let mut w = self.value.write();
        self.sender.send(ChannelMessage::Ack(self.id));
        *w = value;

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

    pub fn read<R>(&self, mut f: impl FnMut(&T) -> R) -> R {
        let r = self.value.read();
        f(&r)
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
pub struct Signal<T, Q: GetQueueType = NoQueue> {
    id: u64,
    sender: MessageSender,
    phantom: PhantomData<(T, Q)>,
}

impl<T: Serialize + Clone, Q: GetQueueType> Signal<T, Q> {
    pub(crate) fn new(id: u64, sender: MessageSender) -> Arc<Self> {
        Arc::new(Self {
            id,
            sender,
            phantom: PhantomData,
        })
    }

    pub fn set(&self, value: impl Into<T>) {
        let message = to_message(&value.into());
        self.sender.send(ChannelMessage::Signal(self.id, message));
    }
}
