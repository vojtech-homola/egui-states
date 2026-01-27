use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;

use egui_states_core::serialization::{deserialize, to_message};

use crate::sender::{ChannelMessage, MessageSender};
use crate::values_atomic::{Atomic, AtomicLock};

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

pub struct DiffAtomic<'a, T: Atomic> {
    pub v: T,
    original: T,
    value: &'a ValueAtomic<T>,
}

impl<'a, T: Serialize + Clone + PartialEq + Atomic> DiffAtomic<'a, T> {
    pub fn new(value: &'a ValueAtomic<T>) -> Self {
        let v = value.get();
        Self {
            v: v,
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

pub trait GetQueueType: Sync + Send + 'static {
    fn is_queue() -> bool;
}

pub struct NoQueue;

impl GetQueueType for NoQueue {
    #[inline]
    fn is_queue() -> bool {
        false
    }
}

pub struct Queue;

impl GetQueueType for Queue {
    #[inline]
    fn is_queue() -> bool {
        true
    }
}

// Value --------------------------------------------
pub struct Value<T, Q: GetQueueType = NoQueue> {
    id: u64,
    value: RwLock<T>,
    sender: MessageSender,
    _phantom: PhantomData<Q>,
}

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

enum ValueLock<T: Atomic> {
    Atomic(Mutex<()>, T::Lock),
    Fallback(RwLock<T>),
}

pub struct ValueAtomic<T: Atomic, Q: GetQueueType = NoQueue> {
    id: u64,
    value: ValueLock<T>,
    sender: MessageSender,
    _phantom: PhantomData<Q>,
}

impl<T, Q: GetQueueType> ValueAtomic<T, Q>
where
    T: Serialize + Clone + Atomic,
{
    pub(crate) fn new(id: u64, value: T, sender: MessageSender) -> Arc<Self> {
        let value = if std::mem::size_of::<T>() > 4 && !cfg!(target_has_atomic = "64") {
            ValueLock::Fallback(RwLock::new(value))
        } else {
            ValueLock::Atomic(Mutex::new(()), T::Lock::new(value))
        };

        Arc::new(Self {
            id,
            value,
            sender,
            _phantom: PhantomData,
        })
    }

    pub fn get(&self) -> T {
        match &self.value {
            ValueLock::Atomic(_, lock) => lock.load(),
            ValueLock::Fallback(rwlock) => *rwlock.read(),
        }
    }

    pub fn set(&self, value: T) {
        let message = ChannelMessage::Value(self.id, false, to_message(&value));
        self.set_raw(value, message);
    }

    pub fn set_signal(&self, value: T) {
        let message = ChannelMessage::Value(self.id, true, to_message(&value));
        self.set_raw(value, message);
    }

    #[inline]
    fn set_raw(&self, value: T, message: ChannelMessage) {
        match &self.value {
            ValueLock::Atomic(lock, val) => {
                let _g = lock.lock();
                self.sender.send(message);
                val.store(value);
            }
            ValueLock::Fallback(rwlock) => {
                let mut w = rwlock.write();
                self.sender.send(message);
                *w = value;
            }
        }
    }
}

impl<T: for<'a> Deserialize<'a> + Atomic + Send + Sync, Q: GetQueueType + Send + Sync> UpdateValue
    for ValueAtomic<T, Q>
{
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        let message = ChannelMessage::Ack(self.id);

        match &self.value {
            ValueLock::Atomic(lock, val) => {
                let _g = lock.lock();
                self.sender.send(message);
                val.store(value);
            }
            ValueLock::Fallback(rwlock) => {
                let mut w = rwlock.write();
                self.sender.send(message);
                *w = value;
            }
        }

        Ok(())
    }
}

// Static --------------------------------------------
pub struct Static<T> {
    id: u64,
    value: RwLock<T>,
}

impl<T: Clone> Static<T> {
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

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateValue for Static<T> {
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;
        *self.value.write() = value;
        Ok(())
    }
}

pub struct StaticAtomic<T: Atomic> {
    id: u64,
    value: T::Lock,
}

impl<T: Atomic> StaticAtomic<T> {
    pub(crate) fn new(id: u64, value: T) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: T::Lock::new(value),
        })
    }

    pub fn get(&self) -> T {
        self.value.load()
    }
}

impl<T: for<'a> Deserialize<'a> + Atomic + Send + Sync> UpdateValue for StaticAtomic<T> {
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;
        self.value.store(value);
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
