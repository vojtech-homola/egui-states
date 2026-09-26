//! Client handles for scalar values, signals, and one-shot values.

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

    /// Sends the edited value if it differs from the captured value.
    ///
    /// The comparison uses `T`'s [`PartialEq`] implementation.
    #[inline]
    pub fn set(self) {
        if self.v != self.original {
            self.value.set(self.v);
        }
    }

    /// Sends and signals the edited value if it differs from the captured value.
    ///
    /// The comparison uses `T`'s [`PartialEq`] implementation.
    #[inline]
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

    /// Sends the edited value if it differs from the captured value.
    ///
    /// The comparison uses `T`'s [`PartialEq`] implementation.
    #[inline]
    pub fn set(self) {
        if self.v != self.original {
            self.value.set(self.v);
        }
    }

    /// Sends and signals the edited value if it differs from the captured value.
    ///
    /// The comparison uses `T`'s [`PartialEq`] implementation.
    #[inline]
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

/// Selects how server callbacks buffer signaled client changes.
pub trait GetQueueType: Sync + Send + 'static {
    /// Returns whether every pending change should be queued.
    fn is_queue() -> bool;
}

/// Coalescing mode: callbacks receive only the latest pending signaled change.
pub struct NoQueue;

impl GetQueueType for NoQueue {
    #[inline]
    fn is_queue() -> bool {
        false
    }
}

/// Queue mode: callbacks receive every signaled change in arrival order.
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
///
/// An oversized [`Self::set`] is rejected before changing the local copy. An
/// oversized in-place [`Self::write`] keeps the local edit but cannot send it,
/// so the peers remain out of sync until a later successful write or server
/// update.
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
    ///
    /// If serialization exceeds the protocol value limit, the operation is
    /// reported and the stored client value is left unchanged.
    pub fn set(&self, value: T) {
        self.set_inner(value, false);
    }

    /// Replaces the client value and asks the server to emit its callbacks.
    ///
    /// If serialization exceeds the protocol value limit, the operation is
    /// reported and the stored client value is left unchanged.
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
///
/// Built-in implementations cover integers, `bool`, `f32`, `f64`, `(f32,
/// f32)`, and `[f32; 2]`. Custom [`Atomic`] implementations must preserve the
/// synchronization and size guarantees documented by that unsafe trait.
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

    /// Emits `value` to the server without retaining it on the client.
    ///
    /// The `Into<T>` parameter accepts values that can be converted to the
    /// signal's payload type, such as `&str` for a `Signal<String>`.
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
    ///
    /// Each value can be taken once. Taking a blocking value also acknowledges
    /// it, allowing the server to send the next pending value.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::VALUE_MAX_SIZE;

    #[test]
    fn diff_and_atomic_diff_emit_only_changes_in_call_order() {
        let (sender, mut rx) = MessageSender::new();
        let value: Value<i32> = Value::new("value".into(), 31, 7, 3, sender.clone());
        let atomic = ValueAtomic::new("atomic".into(), 32, 9, 1.5f64, sender);
        Diff::new(&value).set();
        Diff::new(&value).set_signal();
        DiffAtomic::new(&atomic).set();
        DiffAtomic::new(&atomic).set_signal();
        assert!(rx.try_recv().is_err());
        let mut diff = Diff::new(&value);
        diff.v = -7;
        diff.set_signal();
        let mut diff = DiffAtomic::new(&atomic);
        diff.v = 2.75;
        diff.set();
        match rx.try_recv().unwrap().unwrap() {
            ChannelMessage::Value(31, 7, true, data) => {
                assert_eq!(deserialize::<i32>(&data.to_bytes()).unwrap(), -7)
            }
            _ => panic!("expected signaled scalar first"),
        }
        match rx.try_recv().unwrap().unwrap() {
            ChannelMessage::Value(32, 9, false, data) => {
                assert_eq!(deserialize::<f64>(&data.to_bytes()).unwrap(), 2.75)
            }
            _ => panic!("expected atomic value second"),
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(value.get(), -7);
        assert_eq!(atomic.get(), 2.75);
    }

    #[test]
    fn oversized_set_rejects_but_in_place_write_retains_local_edit() {
        let (sender, mut rx) = MessageSender::new();
        let value: Value<String> = Value::new("text".into(), 1, 2, "seed".into(), sender);
        let oversized = "x".repeat(VALUE_MAX_SIZE + 1);
        value.set(oversized.clone());
        assert_eq!(value.get(), "seed");
        assert!(matches!(
            rx.try_recv(),
            Ok(Some(ChannelMessage::Message(_)))
        ));
        value.write(|v| *v = oversized.clone());
        assert_eq!(value.get(), oversized);
        assert!(matches!(
            rx.try_recv(),
            Ok(Some(ChannelMessage::Message(_)))
        ));
        assert!(rx.try_recv().is_err());
        value.set("recovered".into());
        assert_eq!(value.get(), "recovered");
        assert!(matches!(
            rx.try_recv(),
            Ok(Some(ChannelMessage::Value(1, 2, false, _)))
        ));
        assert!(rx.try_recv().is_err());
    }
}

#[cfg(test)]
mod concurrent_tests {
    use super::*;

    #[test]
    fn concurrent_atomic_sends_match_final_storage_and_each_writers_order() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let value: ValueAtomic<u32> = ValueAtomic::new("atomic".into(), 33, 4, 0, sender);
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let workers: Vec<_> = (0..4)
            .map(|writer| {
                let value = value.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for sequence in 1..=50 {
                        value.set(writer * 1000 + sequence);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let mut previous = [0; 4];
        let mut last = 0;
        for _ in 0..200 {
            match receiver.try_recv().unwrap().unwrap() {
                ChannelMessage::Value(33, 4, false, data) => {
                    let sent: u32 = deserialize(&data.to_bytes()).unwrap();
                    let writer = (sent / 1000) as usize;
                    assert_eq!(sent % 1000, previous[writer] + 1);
                    previous[writer] += 1;
                    last = sent;
                }
                _ => panic!("unexpected outgoing atomic update"),
            }
        }
        assert_eq!(previous, [50; 4]);
        assert_eq!(
            value.get(),
            last,
            "last notification and committed value diverged"
        );
        assert!(receiver.try_recv().is_err());
    }
}
