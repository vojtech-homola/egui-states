use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::client::atomics::{Atomic, AtomicLock, AtomicLockStatic, AtomicStatic};
use crate::client::event::EventUniversal;
use crate::client::sender::{ChannelMessage, MessageSender};
use crate::serialization::{deserialize, to_message};

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
    fn update_value(&self, type_id: u32, data: &[u8]) -> Result<(), String>;
}

pub trait UpdateValueTake: Sync + Send {
    fn update_take(&self, type_id: u32, data: &[u8], blocking: bool) -> Result<(), String>;
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
    name: String,
    id: u64,
    type_id: u32,
    inner: Arc<(RwLock<T>, MessageSender)>,
    _phantom: PhantomData<Q>,
}

impl<T, Q: GetQueueType> Value<T, Q>
where
    T: Serialize + Clone,
{
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        value: T,
        sender: MessageSender,
    ) -> Self {
        Self {
            name,
            id,
            type_id,
            inner: Arc::new((RwLock::new(value), sender)),
            _phantom: PhantomData,
        }
    }

    pub fn get(&self) -> T {
        self.inner.0.read().clone()
    }

    pub fn read<R>(&self, mut f: impl FnMut(&T) -> R) -> R {
        let r = self.inner.0.read();
        f(&r)
    }

    pub fn write<R>(&self, mut f: impl FnMut(&mut T) -> R) -> R {
        let mut w = self.inner.0.write();

        let result = f(&mut w);

        let data = to_message(&*w);
        self.inner
            .1
            .send(ChannelMessage::Value(self.id, self.type_id, false, data));
        result
    }

    pub fn write_signal<R>(&self, mut f: impl FnMut(&mut T) -> R) -> R {
        let mut w = self.inner.0.write();

        let result = f(&mut w);

        let data = to_message(&*w);
        self.inner
            .1
            .send(ChannelMessage::Value(self.id, self.type_id, true, data));
        result
    }

    pub fn set(&self, value: T) {
        let data = to_message(&value);

        let mut w = self.inner.0.write();
        self.inner
            .1
            .send(ChannelMessage::Value(self.id, self.type_id, false, data));
        *w = value;
    }

    pub fn set_signal(&self, value: T) {
        let data = to_message(&value);

        let mut w = self.inner.0.write();
        self.inner
            .1
            .send(ChannelMessage::Value(self.id, self.type_id, true, data));
        *w = value;
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync, Q: GetQueueType + Send + Sync> UpdateValue
    for Value<T, Q>
{
    fn update_value(&self, type_id: u32, data: &[u8]) -> Result<(), String> {
        if type_id != self.type_id {
            self.inner.1.send(ChannelMessage::Ack(self.id));
            return Err(format!("Type id mismatch for Value: {}", self.name));
        }
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value: {}", e, self.name))?;

        let mut w = self.inner.0.write();
        self.inner.1.send(ChannelMessage::Ack(self.id));
        *w = value;

        Ok(())
    }
}

impl<T, Q: GetQueueType> Clone for Value<T, Q> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            inner: self.inner.clone(),
            _phantom: PhantomData,
        }
    }
}

pub struct ValueAtomic<T: Atomic, Q: GetQueueType = NoQueue> {
    name: String,
    id: u64,
    type_id: u32,
    inner: Arc<(T::Lock, MessageSender)>,
    _phantom: PhantomData<Q>,
}

impl<T, Q: GetQueueType> ValueAtomic<T, Q>
where
    T: Serialize + Clone + Atomic,
{
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        value: T,
        sender: MessageSender,
    ) -> Self {
        Self {
            name,
            id,
            type_id,
            inner: Arc::new((T::Lock::new(value), sender)),
            _phantom: PhantomData,
        }
    }

    pub fn get(&self) -> T {
        self.inner.0.load()
    }

    pub fn set(&self, value: T) {
        let message = ChannelMessage::Value(self.id, self.type_id, false, to_message(&value));
        self.inner.0.update(value, || self.inner.1.send(message));
    }

    pub fn set_signal(&self, value: T) {
        let message = ChannelMessage::Value(self.id, self.type_id, true, to_message(&value));
        self.inner.0.update(value, || self.inner.1.send(message));
    }
}

impl<T: for<'a> Deserialize<'a> + Atomic + Send + Sync, Q: GetQueueType + Send + Sync> UpdateValue
    for ValueAtomic<T, Q>
{
    fn update_value(&self, type_id: u32, data: &[u8]) -> Result<(), String> {
        if type_id != self.type_id {
            self.inner.1.send(ChannelMessage::Ack(self.id));
            return Err(format!("Type id mismatch for ValueAtomic: {}", self.name));
        }
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        self.inner
            .0
            .update(value, || self.inner.1.send(ChannelMessage::Ack(self.id)));

        Ok(())
    }
}

impl<T: Atomic, Q: GetQueueType> Clone for ValueAtomic<T, Q> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            inner: self.inner.clone(),
            _phantom: PhantomData,
        }
    }
}

// Static --------------------------------------------
pub struct Static<T> {
    name: String,
    id: u64,
    type_id: u32,
    value: Arc<RwLock<T>>,
}

impl<T: Clone> Static<T> {
    pub(crate) fn new(name: String, id: u64, type_id: u32, value: T) -> Self {
        Self {
            name,
            id,
            type_id,
            value: Arc::new(RwLock::new(value)),
        }
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
    fn update_value(&self, type_id: u32, data: &[u8]) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for Static: {}", self.name));
        }
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value: {}", e, self.name))?;
        *self.value.write() = value;
        Ok(())
    }
}

impl<T> Clone for Static<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            value: self.value.clone(),
        }
    }
}

pub struct StaticAtomic<T: AtomicStatic> {
    name: String,
    id: u64,
    type_id: u32,
    value: Arc<T::Lock>,
}

impl<T: AtomicStatic> StaticAtomic<T> {
    pub(crate) fn new(name: String, id: u64, type_id: u32, value: T) -> Self {
        Self {
            name,
            id,
            type_id,
            value: Arc::new(T::Lock::new(value)),
        }
    }

    pub fn get(&self) -> T {
        self.value.load()
    }
}

impl<T: for<'a> Deserialize<'a> + AtomicStatic + Send + Sync> UpdateValue for StaticAtomic<T> {
    fn update_value(&self, type_id: u32, data: &[u8]) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for AtomicStatic: {}", self.name));
        }
        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value: {}", e, self.name))?;
        self.value.store(value);
        Ok(())
    }
}

impl<T: AtomicStatic> Clone for StaticAtomic<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            value: self.value.clone(),
        }
    }
}

// Signal --------------------------------------------
pub struct Signal<T, Q: GetQueueType = NoQueue> {
    id: u64,
    type_id: u32,
    sender: Arc<MessageSender>,
    phantom: PhantomData<(T, Q)>,
}

impl<T: Serialize + Clone, Q: GetQueueType> Signal<T, Q> {
    pub(crate) fn new(id: u64, type_id: u32, sender: MessageSender) -> Self {
        Self {
            id,
            type_id,
            sender: Arc::new(sender),
            phantom: PhantomData,
        }
    }

    pub fn set(&self, value: impl Into<T>) {
        let message = to_message(&value.into());
        self.sender
            .send(ChannelMessage::Signal(self.id, self.type_id, message));
    }
}

impl<T, Q: GetQueueType> Clone for Signal<T, Q> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            type_id: self.type_id,
            sender: self.sender.clone(),
            phantom: PhantomData,
        }
    }
}

// ValueTake --------------------------------------------
pub struct ValueTake<T> {
    name: String,
    id: u64,
    type_id: u32,
    value: Arc<RwLock<Option<(T, bool)>>>,
    sender: MessageSender,
    event: EventUniversal,
}

impl<T> ValueTake<T> {
    pub(crate) fn new(name: String, id: u64, type_id: u32, sender: MessageSender) -> Self {
        Self {
            name,
            id,
            type_id,
            value: Arc::new(RwLock::new(None)),
            sender,
            event: EventUniversal::new(),
        }
    }

    pub fn take(&self) -> Option<T> {
        let value = self.value.write().take();
        if let Some((val, blocking)) = value {
            if blocking {
                self.sender.send(ChannelMessage::Ack(self.id));
            }
            return Some(val);
        }
        None
    }

    pub fn is_some(&self) -> bool {
        self.value.read().is_some()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait(&self) {
        while self.value.read().is_none() {
            self.event.wait_clear_blocking();
        }
    }

    pub async fn wait_async(&self) {
        while self.value.read().is_none() {
            self.event.wait_clear().await;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_take(&self) -> T {
        loop {
            if let Some((value, blocking)) = self.value.write().take() {
                if blocking {
                    self.sender.send(ChannelMessage::Ack(self.id));
                }
                return value;
            }
            self.event.wait_clear_blocking();
        }
    }

    pub async fn wait_take_async(&self) -> T {
        loop {
            if let Some((value, blocking)) = self.value.write().take() {
                if blocking {
                    self.sender.send(ChannelMessage::Ack(self.id));
                }
                return value;
            }
            self.event.wait_clear().await;
        }
    }
}

impl<T> UpdateValueTake for ValueTake<T>
where
    T: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_take(&self, type_id: u32, data: &[u8], blocking: bool) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for ValueTake: {}", self.name));
        }

        let value = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value: {}", e, self.name))?;
        *self.value.write() = Some((value, blocking));
        self.event.set();

        Ok(())
    }
}

impl<T> Clone for ValueTake<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            value: self.value.clone(),
            sender: self.sender.clone(),
            event: self.event.clone(),
        }
    }
}
