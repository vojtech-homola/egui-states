use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::client::atomics::{Atomic, AtomicLock, AtomicLockStatic, AtomicStatic};
use crate::client::client::print_error;
use crate::client::messages::{ChannelMessage, MessageSender};
use crate::serialization::{check_value_size, deserialize, to_message};

/// Report a value that cannot be sent, locally and to the server.
#[cold]
fn report_error(sender: &MessageSender, error: String) {
    print_error(&error);
    sender.send_message(&error);
}

/// Editable snapshot that sends a [`Value`] only when it changed.
///
/// Modify [`Self::v`], then consume the snapshot with [`Self::set`] or
/// [`Self::set_signal`].
pub struct Diff<'a, T> {
    /// Editable copy of the synchronized value.
    pub v: T,
    original: T,
    value: &'a Value<T>,
}

impl<'a, T: Serialize + Clone + PartialEq> Diff<'a, T> {
    /// Captures the current value and remembers its source handle.
    pub fn new(value: &'a Value<T>) -> Self {
        let v = value.get();
        Self {
            v: v.clone(),
            original: v,
            value,
        }
    }

    #[inline]
    /// Sends the edited value if it differs from the captured value.
    pub fn set(self) {
        if self.v != self.original {
            self.value.set(self.v);
        }
    }

    #[inline]
    /// Sends and signals the edited value if it differs from the captured value.
    pub fn set_signal(self) {
        if self.v != self.original {
            self.value.set_signal(self.v);
        }
    }
}

/// Editable snapshot that sends a [`ValueAtomic`] only when it changed.
pub struct DiffAtomic<'a, T: Atomic> {
    /// Editable copy of the synchronized value.
    pub v: T,
    original: T,
    value: &'a ValueAtomic<T>,
}

impl<'a, T: Serialize + Clone + PartialEq + Atomic> DiffAtomic<'a, T> {
    /// Captures the current value and remembers its source handle.
    pub fn new(value: &'a ValueAtomic<T>) -> Self {
        let v = value.get();
        Self {
            v: v,
            original: v,
            value,
        }
    }

    #[inline]
    /// Sends the edited value if it differs from the captured value.
    pub fn set(self) {
        if self.v != self.original {
            self.value.set(self.v);
        }
    }

    #[inline]
    /// Sends and signals the edited value if it differs from the captured value.
    pub fn set_signal(self) {
        if self.v != self.original {
            self.value.set_signal(self.v);
        }
    }
}

pub(crate) trait UpdateValue: Sync + Send {
    fn update_value(&self, type_id: u32, data: &[u8]) -> Result<(), String>;
}

pub(crate) trait UpdateValueTake: Sync + Send {
    fn update_take(&self, type_id: u32, data: &[u8], blocking: bool) -> Result<(), String>;
}

/// Type-level selection of how incoming server callbacks are buffered.
pub trait GetQueueType: Sync + Send + 'static {
    /// Returns whether every pending change should be queued.
    fn is_queue() -> bool;
}

/// Marker selecting single mode, where pending changes may coalesce.
pub struct NoQueue;

impl GetQueueType for NoQueue {
    #[inline]
    fn is_queue() -> bool {
        false
    }
}

/// Marker selecting queue mode, where every pending change is processed.
pub struct Queue;

impl GetQueueType for Queue {
    #[inline]
    fn is_queue() -> bool {
        true
    }
}

// Value --------------------------------------------
/// A bidirectional synchronized value.
///
/// Client writes update the local copy immediately and are sent to the server.
/// `Q` controls how server-side callbacks process changes produced by
/// [`Self::set_signal`] or [`Self::write_signal`].
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

    /// Returns a copy of the current client value.
    pub fn get(&self) -> T {
        self.inner.0.read().clone()
    }

    /// Borrows the current value for the duration of `f` without copying it.
    pub fn read<R>(&self, f: impl Fn(&T) -> R) -> R {
        let r = self.inner.0.read();
        f(&r)
    }

    fn write_inner(&self, value: &T, signal: bool) {
        let data = to_message(&value);

        if let Err(e) = check_value_size(&self.name, data.len()) {
            report_error(
                &self.inner.1,
                format!("{}, the value stays out of sync with the server", e),
            );
            return;
        }

        self.inner
            .1
            .send(ChannelMessage::Value(self.id, self.type_id, signal, data));
    }

    /// Modify the value in place and send it to the server.
    ///
    /// If the modified value serializes to more than the maximum allowed size, it is
    /// kept locally but not sent, which leaves it out of sync with the server until
    /// a later write succeeds or the server sends an update. The error is reported
    /// locally and to the server.
    pub fn write<R>(&self, f: impl Fn(&mut T) -> R) -> R {
        let mut w = self.inner.0.write();
        let result = f(&mut w);
        self.write_inner(&*w, false);
        result
    }

    /// Same as [`Value::write`], but the server side emits a signal for the new value.
    ///
    /// The same out of sync caveat for oversized values applies.
    pub fn write_signal<R>(&self, f: impl Fn(&mut T) -> R) -> R {
        let mut w = self.inner.0.write();
        let result = f(&mut w);
        self.write_inner(&*w, true);
        result
    }

    #[inline]
    fn set_inner(&self, value: T, signal: bool) {
        let data = to_message(&value);

        if let Err(e) = check_value_size(&self.name, data.len()) {
            report_error(&self.inner.1, format!("{}, the value was not set", e));
            return;
        }

        let mut w = self.inner.0.write();
        self.inner
            .1
            .send(ChannelMessage::Value(self.id, self.type_id, signal, data));
        *w = value;
    }

    /// Replaces the client value and sends it to the server without signaling.
    pub fn set(&self, value: T) {
        self.set_inner(value, false);
    }

    /// Replaces the client value and asks the server to emit its callbacks.
    pub fn set_signal(&self, value: T) {
        self.set_inner(value, true);
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
        let value = deserialize(data).map_err(|e| {
            self.inner.1.send(ChannelMessage::Ack(self.id));
            format!("Parse error: {} for value: {}", e, self.name)
        })?;

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

/// Atomic variant of [`Value`] for small copyable values.
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

    /// Atomically loads the current client value.
    pub fn get(&self) -> T {
        self.inner.0.load()
    }

    fn set_inner(&self, value: T, signal: bool) {
        let data = to_message(&value);

        // the built in atomic types are always small, but `Atomic` is public and
        // hand written implementations are not limited in size
        if let Err(e) = check_value_size(&self.name, data.len()) {
            report_error(&self.inner.1, format!("{}, the value was not set", e));
            return;
        }

        let message = ChannelMessage::Value(self.id, self.type_id, signal, data);
        self.inner.0.update(value, || self.inner.1.send(message));
    }

    /// Atomically replaces the value and sends it without signaling.
    pub fn set(&self, value: T) {
        self.set_inner(value, false);
    }

    /// Atomically replaces the value and asks the server to emit callbacks.
    pub fn set_signal(&self, value: T) {
        self.set_inner(value, true);
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
        let value = deserialize(data).map_err(|e| {
            self.inner.1.send(ChannelMessage::Ack(self.id));
            format!("Parse error: {} for value id: {}", e, self.id)
        })?;

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
/// A server-controlled value that is read-only on the client.
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

    /// Returns a copy of the current value.
    pub fn get(&self) -> T {
        self.value.read().clone()
    }

    /// Borrows the current value for the duration of `f` without copying it.
    pub fn read<R>(&self, f: impl Fn(&T) -> R) -> R {
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

/// Atomic server-controlled value that is read-only on the client.
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

    /// Atomically loads the current value.
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
/// A client-to-server event carrying a value without storing it.
///
/// `Q` controls whether server callbacks queue every event or coalesce pending
/// events to the latest one.
pub struct Signal<T, Q: GetQueueType = NoQueue> {
    name: String,
    id: u64,
    type_id: u32,
    sender: Arc<MessageSender>,
    phantom: PhantomData<(T, Q)>,
}

impl<T: Serialize + Clone, Q: GetQueueType> Signal<T, Q> {
    pub(crate) fn new(name: String, id: u64, type_id: u32, sender: MessageSender) -> Self {
        Self {
            name,
            id,
            type_id,
            sender: Arc::new(sender),
            phantom: PhantomData,
        }
    }

    /// Emits `value` to the server.
    pub fn set(&self, value: impl Into<T>) {
        let message = to_message(&value.into());

        if let Err(e) = check_value_size(&self.name, message.len()) {
            report_error(&self.sender, format!("{}, the signal was not sent", e));
            return;
        }

        self.sender
            .send(ChannelMessage::Signal(self.id, self.type_id, message));
    }
}

impl<T, Q: GetQueueType> Clone for Signal<T, Q> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            sender: self.sender.clone(),
            phantom: PhantomData,
        }
    }
}

// ValueTake --------------------------------------------
/// A one-shot value sent by the server and consumed by the client.
///
/// A newly received value replaces an untaken value. Taking a blocking value
/// sends the acknowledgement that releases the server's next send.
pub struct ValueTake<T> {
    name: String,
    id: u64,
    type_id: u32,
    value: Arc<RwLock<Option<(T, bool)>>>,
    sender: MessageSender,
}

impl<T> ValueTake<T> {
    pub(crate) fn new(name: String, id: u64, type_id: u32, sender: MessageSender) -> Self {
        Self {
            name,
            id,
            type_id,
            value: Arc::new(RwLock::new(None)),
            sender,
        }
    }

    /// Removes and returns the pending value, if one has arrived.
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

    /// Returns whether a value is waiting to be taken.
    pub fn is_some(&self) -> bool {
        self.value.read().is_some()
    }
}

impl<T> UpdateValueTake for ValueTake<T>
where
    T: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_take(&self, type_id: u32, data: &[u8], blocking: bool) -> Result<(), String> {
        if type_id != self.type_id {
            if blocking {
                self.sender.send(ChannelMessage::Ack(self.id));
            }
            return Err(format!("Type id mismatch for ValueTake: {}", self.name));
        }

        let value = deserialize(data).map_err(|e| {
            if blocking {
                self.sender.send(ChannelMessage::Ack(self.id));
            }
            format!("Parse error: {} for value: {}", e, self.name)
        })?;
        *self.value.write() = Some((value, blocking));

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
        }
    }
}
