use std::marker::PhantomData;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::Typed;
use crate::server_core::values_core::{SignalCore, ValueCore, ValueStaticCore, ValueTakeCore};

use super::callbacks::CallbackHandle;
use super::state_server::{StateServer, deserialize_bytes, serialize_bytes};
use super::{Result, ServerError};

#[derive(Clone)]
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
    pub fn set(&self, value: T, update: bool) -> Result<()> {
        self.set_with_signal(value, false, update)
    }

    pub fn set_signal(&self, value: T, update: bool) -> Result<()> {
        self.set_with_signal(value, true, update)
    }

    pub fn set_with_signal(&self, value: T, set_signal: bool, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner
            .set(data, set_signal, update)
            .map_err(ServerError::new)
    }
}

impl<T> Value<T> {
    pub fn signal_set_to_queue(&self) {
        self.server.set_signal_to_queue(self.id);
    }

    pub fn signal_set_to_single(&self) {
        self.server.set_signal_to_single(self.id);
    }
}

impl<T> Value<T>
where
    T: for<'a> Deserialize<'a> + Send + 'static,
{
    pub fn get(&self) -> Result<T> {
        deserialize_bytes(&self.inner.get())
    }

    pub fn connect(&self, callback: impl Fn(T) + Send + Sync + 'static) -> CallbackHandle {
        self.server.add_typed_callback(self.id, callback)
    }
}

#[derive(Clone)]
pub struct Static<T> {
    inner: Arc<ValueStaticCore>,
    _type: PhantomData<T>,
}

impl<T> Static<T>
where
    T: Serialize + Typed,
{
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
    pub fn set(&self, value: T, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner.set(data, update).map_err(ServerError::new)
    }
}

impl<T> Static<T>
where
    T: for<'a> Deserialize<'a>,
{
    pub fn get(&self) -> Result<T> {
        deserialize_bytes(&self.inner.get())
    }
}

#[derive(Clone)]
pub struct ValueTake<T> {
    inner: Arc<ValueTakeCore>,
    _type: PhantomData<T>,
}

impl<T> ValueTake<T>
where
    T: Typed,
{
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
    pub fn set(&self, value: T, blocking: bool, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner
            .set(data, blocking, update)
            .map_err(ServerError::new)
    }
}

#[derive(Clone)]
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
    pub fn set(&self, value: T) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner.set(data);
        Ok(())
    }
}

impl<T> Signal<T> {
    pub fn signal_set_to_queue(&self) {
        self.server.set_signal_to_queue(self.id);
    }

    pub fn signal_set_to_single(&self) {
        self.server.set_signal_to_single(self.id);
    }
}

impl<T> Signal<T>
where
    T: for<'a> Deserialize<'a> + Send + 'static,
{
    pub fn connect(&self, callback: impl Fn(T) + Send + Sync + 'static) -> CallbackHandle {
        self.server.add_typed_callback(self.id, callback)
    }
}

impl Signal<()> {
    pub fn connect_empty(&self, callback: impl Fn() + Send + Sync + 'static) -> CallbackHandle {
        self.server.add_raw_callback(self.id, move |_| {
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
