use std::marker::PhantomData;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::Typed;
use crate::server_core::values_core::{SignalCore, ValueCore, ValueStaticCore, ValueTakeCore};

use super::callbacks::CallbackHandle;
use super::state_server::{StateServer, deserialize_bytes, serialize_bytes};
use super::{Result, ServerError};

#[derive(Clone)]
/// Server-side handle for a bidirectional synchronized value.
pub struct Value<T> {
    server: StateServer,
    id: u64,
    inner: Arc<ValueCore>,
    _type: PhantomData<T>,
}

impl<T> Value<T>
where
    T: Serialize + Typed,
{
    /// Registers `name` with `initial_value` and callback queueing mode.
    ///
    /// Set `queue` to `true` to preserve every signaled client change; in
    /// single mode pending changes may coalesce to the latest value.
    pub fn new(
        server: &StateServer,
        name: impl Into<String>,
        initial_value: T,
        queue: bool,
    ) -> Result<Self> {
        let (id, inner) = server.add_value(name.into(), initial_value, queue)?;
        Ok(Self {
            server: server.clone(),
            id,
            inner,
            _type: PhantomData,
        })
    }
}

impl<T> Value<T>
where
    T: Serialize,
{
    /// Replaces the value without emitting callbacks.
    ///
    /// If `update` is true, the client is also asked to repaint.
    pub fn set(&self, value: T, update: bool) -> Result<()> {
        self.set_with_signal(value, false, update)
    }

    /// Replaces the value, emits callbacks, and optionally requests a repaint.
    pub fn set_signal(&self, value: T, update: bool) -> Result<()> {
        self.set_with_signal(value, true, update)
    }

    /// Replaces the value with explicit callback and repaint controls.
    pub fn set_with_signal(&self, value: T, set_signal: bool, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner
            .set(data, set_signal, update)
            .map_err(ServerError::new)
    }
}

impl<T> Value<T> {
    /// Switches callback delivery to queue mode.
    pub fn signal_set_to_queue(&self) {
        self.server.set_signal_to_queue(self.id);
    }

    /// Switches callback delivery to coalescing single mode.
    pub fn signal_set_to_single(&self) {
        self.server.set_signal_to_single(self.id);
    }
}

impl<T> Value<T>
where
    T: for<'a> Deserialize<'a> + Send + 'static,
{
    /// Returns the server's current value.
    pub fn get(&self) -> Result<T> {
        deserialize_bytes(&self.inner.get())
    }

    /// Registers a callback for signaled changes from either peer.
    ///
    /// Retain the returned handle to keep the callback connected.
    pub fn connect(&self, callback: impl Fn(T) + Send + Sync + 'static) -> CallbackHandle {
        self.server.add_typed_callback(self.id, callback)
    }

    /// Connects a callback that also receives the value being replaced.
    ///
    /// The previous value is the one the callback was last notified about, not
    /// necessarily the immediate predecessor: in single mode successive changes
    /// coalesce, so `a -> b -> c` delivering only `c` reports `(c, a)`.
    pub fn connect_previous(
        &self,
        callback: impl Fn(T, T) + Send + Sync + 'static,
    ) -> CallbackHandle {
        self.server.add_typed_callback_previous(self.id, callback)
    }
}

#[derive(Clone)]
/// Server-controlled value that is read-only on the client.
pub struct Static<T> {
    inner: Arc<ValueStaticCore>,
    _type: PhantomData<T>,
}

impl<T> Static<T>
where
    T: Serialize + Typed,
{
    /// Registers `name` with `initial_value`.
    pub fn new(server: &StateServer, name: impl Into<String>, initial_value: T) -> Result<Self> {
        let (_, inner) = server.add_static(name.into(), initial_value)?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }
}

impl<T> Static<T>
where
    T: Serialize,
{
    /// Replaces the value and optionally asks the client to repaint.
    pub fn set(&self, value: T, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner.set(data, update).map_err(ServerError::new)
    }
}

impl<T> Static<T>
where
    T: for<'a> Deserialize<'a>,
{
    /// Returns the server's current value.
    pub fn get(&self) -> Result<T> {
        deserialize_bytes(&self.inner.get())
    }
}

#[derive(Clone)]
/// One-shot value sent by the server and consumed by the client.
pub struct ValueTake<T> {
    inner: Arc<ValueTakeCore>,
    _type: PhantomData<T>,
}

impl<T> ValueTake<T>
where
    T: Typed,
{
    /// Registers a one-shot value under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_value_take::<T>(name.into())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }
}

impl<T> ValueTake<T>
where
    T: Serialize,
{
    /// Sends a value to the client.
    ///
    /// With `blocking`, a subsequent send waits until the client takes and
    /// acknowledges this value. `update` requests an egui repaint.
    pub fn set(&self, value: T, blocking: bool, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner
            .set(data, blocking, update)
            .map_err(ServerError::new)
    }
}

#[derive(Clone)]
/// Client-to-server event that does not retain a value.
pub struct Signal<T> {
    server: StateServer,
    id: u64,
    inner: Arc<SignalCore>,
    _type: PhantomData<T>,
}

impl<T> Signal<T>
where
    T: Typed,
{
    /// Registers an event and chooses queue or coalescing single mode.
    pub fn new(server: &StateServer, name: impl Into<String>, queue: bool) -> Result<Self> {
        let (id, inner) = server.add_signal::<T>(name.into(), queue)?;
        Ok(Self {
            server: server.clone(),
            id,
            inner,
            _type: PhantomData,
        })
    }
}

impl<T> Signal<T>
where
    T: Serialize,
{
    /// Emits an event from the server to local callbacks.
    pub fn set(&self, value: T) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner.set(data);
        Ok(())
    }
}

impl<T> Signal<T> {
    /// Switches callback delivery to queue mode.
    pub fn signal_set_to_queue(&self) {
        self.server.set_signal_to_queue(self.id);
    }

    /// Switches callback delivery to coalescing single mode.
    pub fn signal_set_to_single(&self) {
        self.server.set_signal_to_single(self.id);
    }
}

impl<T> Signal<T>
where
    T: for<'a> Deserialize<'a> + Send + 'static,
{
    /// Registers a value-taking callback and returns its owning handle.
    pub fn connect(&self, callback: impl Fn(T) + Send + Sync + 'static) -> CallbackHandle {
        self.server.add_typed_callback(self.id, callback)
    }
}

impl Signal<()> {
    /// Registers a no-argument callback for a unit-valued signal.
    pub fn connect_empty(&self, callback: impl Fn() + Send + Sync + 'static) -> CallbackHandle {
        self.server.add_raw_callback(self.id, move |_, _| {
            callback();
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::VALUE_MAX_SIZE;

    #[test]
    fn values_round_trip_without_a_client() {
        let server = StateServer::new(0).unwrap();
        let value = Value::new(&server, "root.value", String::from("initial"), false).unwrap();
        let static_value = Static::new(&server, "root.static", 10_i32).unwrap();

        assert_eq!(value.get().unwrap(), "initial");
        value.set(String::from("changed"), false).unwrap();
        assert_eq!(value.get().unwrap(), "changed");

        assert_eq!(static_value.get().unwrap(), 10);
        static_value.set(20, false).unwrap();
        assert_eq!(static_value.get().unwrap(), 20);
    }

    #[test]
    fn oversized_values_are_rejected_without_a_client() {
        let server = StateServer::new(0).unwrap();
        let value = Value::new(&server, "root.value", String::from("initial"), false).unwrap();
        let static_value = Static::new(&server, "root.static", String::from("initial")).unwrap();

        // no client is connected, but the value would be sent by sync() after a handshake
        let too_large = "a".repeat(VALUE_MAX_SIZE + 1);

        let error = value.set(too_large.clone(), false).unwrap_err();
        assert!(error.message().contains("root.value"), "{error}");
        assert_eq!(value.get().unwrap(), "initial");

        let error = static_value.set(too_large, false).unwrap_err();
        assert!(error.message().contains("root.static"), "{error}");
        assert_eq!(static_value.get().unwrap(), "initial");
    }

    #[test]
    fn oversized_initial_value_is_rejected() {
        let server = StateServer::new(0).unwrap();
        let too_large = "a".repeat(VALUE_MAX_SIZE + 1);

        assert!(Value::new(&server, "root.value", too_large.clone(), false).is_err());
        assert!(Static::new(&server, "root.static", too_large).is_err());
    }

    #[test]
    fn state_ids_are_unique_across_state_kinds() {
        let server = StateServer::new(0).unwrap();
        Value::new(&server, "root.duplicate", 1_i32, false).unwrap();

        assert!(Static::new(&server, "root.duplicate", 1_i32).is_err());
    }
}
