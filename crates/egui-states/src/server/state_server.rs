//! Native server lifecycle and callback-worker management.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::Typed;
use crate::serialization::{deserialize_value, serialize};
use crate::server_core::data_core::{Data as CoreData, DataMulti as CoreDataMulti};
use crate::server_core::data_take_core::{
    DataMultiTake as CoreDataMultiTake, DataTake as CoreDataTake,
};
use crate::server_core::image_core::Image as CoreImage;
use crate::server_core::image_multi_core::ImageMulti as CoreImageMulti;
use crate::server_core::map_core::ValueMap as CoreMap;
use crate::server_core::server::Server as CoreServer;
use crate::server_core::signals::{
    CLIENT_MESSAGE_ID, LOGGING_ID, ON_CONNECT_ID, ON_DISCONNECT_ID,
    SignalsManager as CoreSignalsManager,
};
use crate::server_core::values_core::{
    SignalCore as CoreSignal, ValueCore as CoreValue, ValueStaticCore as CoreStatic,
    ValueTakeCore as CoreValueTake,
};
use crate::server_core::vec_core::ValueList as CoreVec;

use super::callbacks::{CallbackHandle, CallbackRegistry};
use super::data::DataElement;
use super::options::ErrorHandler;
use super::{Result, ServerError, ServerOptions};

#[derive(Clone)]
/// Native WebSocket server that owns synchronized states and callback workers.
///
/// The lifecycle is: register every state, call [`Self::finalize`], call
/// [`Self::start`] with connection settings, and finally call [`Self::stop`] or
/// drop the last clone. A stopped server may be restarted with different
/// connection settings.
/// Generated server bindings perform registration and finalization for you.
pub struct StateServer {
    inner: Arc<ServerInner>,
}

pub(super) struct ServerInner {
    pub(super) server: RwLock<CoreServer>,
    pub(super) signals: CoreSignalsManager,
    pub(super) callbacks: CallbackRegistry,
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    /// Disconnects once every signal worker has dropped its sender, i.e. once
    /// all of them have returned. Used to bound teardown without `join`ing.
    worker_exit: Mutex<Option<mpsc::Receiver<()>>>,
    worker_count: usize,
    worker_shutdown: Arc<AtomicBool>,
    shutdown_timeout: Duration,
    error_handler: RwLock<ErrorHandler>,
}

impl ServerInner {
    fn handle_error(&self, error: ServerError) {
        let handler = self.error_handler.read().clone();
        let _ = catch_unwind(AssertUnwindSafe(|| handler(error)));
    }
}

impl Drop for ServerInner {
    fn drop(&mut self) {
        self.worker_shutdown.store(true, Ordering::Release);
        self.signals.wake_waiters();
        self.server.get_mut().stop();

        // A worker only observes `worker_shutdown` between callbacks, so one
        // that is inside a blocking callback cannot be waited on unboundedly --
        // `Drop` must not hang. Wait for all of them to return, then detach
        // whatever is left. Detaching is safe: a worker holds only a `Weak` to
        // this struct plus `Arc` clones (signals, shutdown flag, the callback
        // itself), so it stays sound after this struct is gone.
        let all_workers_returned = match self.worker_exit.get_mut().take() {
            // Nobody ever sends, so `Disconnected` means every worker has
            // dropped its sender and returned.
            Some(exit) => matches!(
                exit.recv_timeout(self.shutdown_timeout),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ),
            None => true,
        };

        let workers = self.workers.get_mut().drain(..).collect::<Vec<_>>();
        if !all_workers_returned {
            return;
        }

        // Every worker has returned, so these joins are non-blocking. Skip the
        // current thread: `drop` can run on a worker when it releases the last
        // strong reference, and joining yourself is not allowed.
        let current_thread = thread::current().id();
        for worker in workers {
            if worker.thread().id() != current_thread {
                let _ = worker.join();
            }
        }
    }
}

impl StateServer {
    /// Creates a server with default options.
    pub fn new() -> Result<Self> {
        Self::with_options(ServerOptions::new())
    }

    /// Creates a server with explicit application and worker options.
    pub fn with_options(options: ServerOptions) -> Result<Self> {
        let server = CoreServer::new(options.version);
        let signals = server.get_signals_manager();
        signals.set_to_queue(LOGGING_ID);
        signals.set_to_queue(ON_CONNECT_ID);
        signals.set_to_queue(ON_DISCONNECT_ID);
        signals.set_to_queue(CLIENT_MESSAGE_ID);

        let error_handler = options.error_handler.unwrap_or_else(|| {
            Arc::new(|error: ServerError| {
                eprintln!("egui-states server callback error: {error}");
            })
        });

        Ok(Self {
            inner: Arc::new(ServerInner {
                server: RwLock::new(server),
                signals,
                callbacks: CallbackRegistry::new(),
                workers: Mutex::new(Vec::new()),
                worker_exit: Mutex::new(None),
                worker_count: options.signal_workers.max(1),
                shutdown_timeout: options.shutdown_timeout,
                worker_shutdown: Arc::new(AtomicBool::new(false)),
                error_handler: RwLock::new(error_handler),
            }),
        })
    }

    /// Freezes state registration and prepares the handshake description.
    ///
    /// Call this after constructing every state and before [`Self::start`]. A
    /// later attempt to register another state returns an error.
    pub fn finalize(&self) -> Result<()> {
        self.inner.server.write().finalize();
        Ok(())
    }

    /// Starts listening and launches callback workers.
    ///
    /// `ip_addr` selects a specific IPv4 interface; `None` binds all IPv4
    /// interfaces. `token` optionally requires clients to authenticate during
    /// the handshake. After [`Self::stop`], the server may be started again
    /// with different connection settings. Calling this while the server is
    /// already running succeeds without changing its active settings.
    ///
    /// The native server owns a dedicated thread and Tokio runtime; call this
    /// from synchronous application code rather than from code that requires
    /// server shutdown to run inside an existing async runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the server was not finalized, the socket cannot be
    /// bound or configured, or its runtime/thread cannot be started.
    pub fn start(&self, port: u16, ip_addr: Option<Ipv4Addr>, token: Option<String>) -> Result<()> {
        let addr = SocketAddrV4::new(ip_addr.unwrap_or(Ipv4Addr::UNSPECIFIED), port);
        self.inner
            .server
            .write()
            .start(addr, token)
            .map_err(ServerError::new)?;
        self.start_signal_workers();
        Ok(())
    }

    /// Stops listening and disconnects the current client.
    pub fn stop(&self) {
        self.inner.server.write().stop();
    }

    /// Disconnects the current client while leaving the server running.
    pub fn disconnect_client(&self) {
        self.inner.server.write().disconnect_client();
    }

    /// Returns whether the server is listening.
    pub fn is_running(&self) -> bool {
        self.inner.server.read().is_running()
    }

    /// Returns whether a client is connected.
    pub fn is_connected(&self) -> bool {
        self.inner.server.read().is_connected()
    }

    /// Requests a client repaint.
    ///
    /// `None` requests an immediate repaint; `Some(seconds)` schedules it after
    /// the supplied delay. The request is a no-op when no client is connected.
    pub fn update(&self, duration: Option<f32>) -> Result<()> {
        self.inner
            .server
            .read()
            .update(duration)
            .map_err(|_| ServerError::new("failed to send update"))
    }

    /// Replaces the handler for callback decoding failures and callback panics.
    pub fn set_error_handler(&self, handler: impl Fn(ServerError) + Send + Sync + 'static) {
        *self.inner.error_handler.write() = Arc::new(handler);
    }

    /// Registers a callback receiving the remote address after a client connects.
    ///
    /// Retain the returned handle; dropping it unregisters the callback.
    pub fn on_connect(&self, callback: impl Fn(String) + Send + Sync + 'static) -> CallbackHandle {
        self.add_typed_callback(ON_CONNECT_ID, callback)
    }

    /// Registers a callback invoked when the client disconnects.
    ///
    /// Retain the returned handle; dropping it unregisters the callback.
    pub fn on_disconnect(&self, callback: impl Fn() + Send + Sync + 'static) -> CallbackHandle {
        self.add_raw_callback(ON_DISCONNECT_ID, move |_, _| {
            callback();
            Ok(())
        })
    }

    /// Registers a callback for diagnostic text messages sent by the client.
    ///
    /// Retain the returned handle; dropping it unregisters the callback.
    pub fn on_client_message(
        &self,
        callback: impl Fn(String) + Send + Sync + 'static,
    ) -> CallbackHandle {
        self.add_typed_callback(CLIENT_MESSAGE_ID, callback)
    }

    pub(super) fn add_typed_callback<T>(
        &self,
        value_id: u64,
        callback: impl Fn(T) + Send + Sync + 'static,
    ) -> CallbackHandle
    where
        T: for<'a> Deserialize<'a> + Send + 'static,
    {
        self.add_raw_callback(value_id, move |data, _previous| {
            let value = deserialize_bytes::<T>(&data)?;
            callback(value);
            Ok(())
        })
    }

    /// Like [`Self::add_typed_callback`], but the callback also receives the value that
    /// was replaced. Both are decoded as `T`: a state carries one `type_id`, so the new
    /// and previous values always share a type.
    pub(super) fn add_typed_callback_previous<T>(
        &self,
        value_id: u64,
        callback: impl Fn(T, T) + Send + Sync + 'static,
    ) -> CallbackHandle
    where
        T: for<'a> Deserialize<'a> + Send + 'static,
    {
        self.add_raw_callback_previous(value_id, move |data, previous| {
            let Some(previous) = previous else {
                return Ok(());
            };
            let value = deserialize_bytes::<T>(&data)?;
            let previous = deserialize_bytes::<T>(&previous)?;
            callback(value, previous);
            Ok(())
        })
    }

    pub(super) fn add_raw_callback(
        &self,
        value_id: u64,
        callback: impl Fn(Bytes, Option<Bytes>) -> Result<()> + Send + Sync + 'static,
    ) -> CallbackHandle {
        self.add_callback_inner(value_id, callback, false)
    }

    fn add_raw_callback_previous(
        &self,
        value_id: u64,
        callback: impl Fn(Bytes, Option<Bytes>) -> Result<()> + Send + Sync + 'static,
    ) -> CallbackHandle {
        self.add_callback_inner(value_id, callback, true)
    }

    fn add_callback_inner(
        &self,
        value_id: u64,
        callback: impl Fn(Bytes, Option<Bytes>) -> Result<()> + Send + Sync + 'static,
        wants_previous: bool,
    ) -> CallbackHandle {
        let callback_id = self.inner.callbacks.add(
            &self.inner.signals,
            value_id,
            Arc::new(callback),
            wants_previous,
        );
        CallbackHandle {
            server: Arc::downgrade(&self.inner),
            value_id,
            callback_id,
        }
    }

    fn start_signal_workers(&self) {
        let mut workers = self.inner.workers.lock();
        if !workers.is_empty() {
            return;
        }

        let (exit_sender, exit_receiver) = mpsc::channel();
        for index in 0..self.inner.worker_count {
            let inner = Arc::downgrade(&self.inner);
            let signals = self.inner.signals.clone();
            let shutdown = self.inner.worker_shutdown.clone();
            let exit = exit_sender.clone();
            let worker = thread::Builder::new()
                .name(format!("egui_states_signal_worker_{index}"))
                .spawn(move || {
                    // Dropped when the worker returns, which is how `Drop`
                    // learns that every worker has finished.
                    let _exit = exit;
                    run_signal_worker(inner, signals, shutdown)
                });
            match worker {
                Ok(worker) => workers.push(worker),
                Err(error) => self.inner.handle_error(ServerError::new(format!(
                    "failed to start signal worker: {error}"
                ))),
            }
        }
        // Only the workers may hold a sender, otherwise the channel never
        // disconnects and teardown always waits for the full timeout.
        drop(exit_sender);

        if !workers.is_empty() {
            *self.inner.worker_exit.lock() = Some(exit_receiver);
        }
    }

    pub(super) fn add_value<T>(
        &self,
        name: String,
        initial_value: T,
        queue: bool,
    ) -> Result<(u64, Arc<CoreValue>)>
    where
        T: Serialize + Typed,
    {
        let data = serialize_bytes(&initial_value)?;
        let type_id = T::get_type().get_hash();
        let id = self
            .inner
            .server
            .write()
            .add_value(&name, type_id, data, queue)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_value(id).ok_or_else(|| {
            ServerError::new(format!("value not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_value_take<T>(&self, name: String) -> Result<(u64, Arc<CoreValueTake>)>
    where
        T: Typed,
    {
        let type_id = T::get_type().get_hash();
        let id = self
            .inner
            .server
            .write()
            .add_value_take(&name, type_id)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_value_take(id).ok_or_else(|| {
            ServerError::new(format!("value_take not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_static<T>(
        &self,
        name: String,
        initial_value: T,
    ) -> Result<(u64, Arc<CoreStatic>)>
    where
        T: Serialize + Typed,
    {
        let data = serialize_bytes(&initial_value)?;
        let type_id = T::get_type().get_hash();
        let id = self
            .inner
            .server
            .write()
            .add_static(&name, type_id, data)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_static(id).ok_or_else(|| {
            ServerError::new(format!("static not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_signal<T>(&self, name: String, queue: bool) -> Result<(u64, Arc<CoreSignal>)>
    where
        T: Typed,
    {
        let type_id = T::get_type().get_hash();
        let id = self
            .inner
            .server
            .write()
            .add_signal(&name, type_id, queue)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_signal(id).ok_or_else(|| {
            ServerError::new(format!("signal not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_vec<T>(&self, name: String) -> Result<(u64, Arc<CoreVec>)>
    where
        T: Typed,
    {
        let type_id = T::get_type().get_hash();
        let id = self
            .inner
            .server
            .write()
            .add_vec(&name, type_id)
            .map_err(ServerError::new)?;
        let value =
            self.inner.server.read().get_vec(id).ok_or_else(|| {
                ServerError::new(format!("vec not found after registration: {name}"))
            })?;
        Ok((id, value))
    }

    pub(super) fn add_map<K, V>(&self, name: String) -> Result<(u64, Arc<CoreMap>)>
    where
        K: Typed,
        V: Typed,
    {
        let type_id = V::get_type().get_hash_from(K::get_type().get_hash());
        let id = self
            .inner
            .server
            .write()
            .add_map(&name, type_id)
            .map_err(ServerError::new)?;
        let value =
            self.inner.server.read().get_map(id).ok_or_else(|| {
                ServerError::new(format!("map not found after registration: {name}"))
            })?;
        Ok((id, value))
    }

    pub(super) fn add_image(&self, name: String) -> Result<(u64, Arc<CoreImage>)> {
        let id = self
            .inner
            .server
            .write()
            .add_image(&name)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_image(id).ok_or_else(|| {
            ServerError::new(format!("image not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_image_multi(&self, name: String) -> Result<(u64, Arc<CoreImageMulti>)> {
        let id = self
            .inner
            .server
            .write()
            .add_image_multi(&name)
            .map_err(ServerError::new)?;
        let value = self
            .inner
            .server
            .read()
            .get_image_multi(id)
            .ok_or_else(|| {
                ServerError::new(format!("image_multi not found after registration: {name}"))
            })?;
        Ok((id, value))
    }

    pub(super) fn add_data<T>(&self, name: String) -> Result<(u64, Arc<CoreData>)>
    where
        T: DataElement,
    {
        let id = self
            .inner
            .server
            .write()
            .add_data(&name, T::TYPE_ID)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_data(id).ok_or_else(|| {
            ServerError::new(format!("data not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_data_take<T>(&self, name: String) -> Result<(u64, Arc<CoreDataTake>)>
    where
        T: DataElement,
    {
        let id = self
            .inner
            .server
            .write()
            .add_data_take(&name, T::TYPE_ID)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_data_take(id).ok_or_else(|| {
            ServerError::new(format!("data_take not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_data_multi<T>(&self, name: String) -> Result<(u64, Arc<CoreDataMulti>)>
    where
        T: DataElement,
    {
        let id = self
            .inner
            .server
            .write()
            .add_data_multi(&name, T::TYPE_ID)
            .map_err(ServerError::new)?;
        let value = self.inner.server.read().get_data_multi(id).ok_or_else(|| {
            ServerError::new(format!("data_multi not found after registration: {name}"))
        })?;
        Ok((id, value))
    }

    pub(super) fn add_data_multi_take<T>(
        &self,
        name: String,
    ) -> Result<(u64, Arc<CoreDataMultiTake>)>
    where
        T: DataElement,
    {
        let id = self
            .inner
            .server
            .write()
            .add_data_multi_take(&name, T::TYPE_ID)
            .map_err(ServerError::new)?;
        let value = self
            .inner
            .server
            .read()
            .get_data_multi_take(id)
            .ok_or_else(|| {
                ServerError::new(format!(
                    "data_multi_take not found after registration: {name}"
                ))
            })?;
        Ok((id, value))
    }

    pub(super) fn set_signal_to_queue(&self, id: u64) {
        self.inner.signals.set_to_queue(id);
    }

    pub(super) fn set_signal_to_single(&self, id: u64) {
        self.inner.signals.set_to_single(id);
    }
}

fn run_signal_worker(
    inner: Weak<ServerInner>,
    signals: CoreSignalsManager,
    shutdown: Arc<AtomicBool>,
) {
    let mut last_id = None;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return;
        }

        // `take` matters: the id is released exactly once, on the poll that
        // follows its dispatch. Retrying with it still set would release a
        // claim another worker may have taken in the meantime, letting two
        // workers run callbacks for the same value concurrently.
        let Some((value_id, data, previous)) = signals.try_changed_value(last_id.take()) else {
            if !signals.wait_for_change_until(&shutdown) {
                return;
            }
            continue;
        };
        last_id = Some(value_id);

        let Some(server) = inner.upgrade() else {
            return;
        };
        let callbacks = server.callbacks.get(value_id);
        let error_handler = server.error_handler.read().clone();
        drop(server);

        for entry in callbacks {
            let data = data.clone();
            let previous = previous.clone();
            let result = catch_unwind(AssertUnwindSafe(|| (entry.callback)(data, previous)));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => handle_error(&error_handler, error),
                Err(_) => handle_error(
                    &error_handler,
                    ServerError::new(format!(
                        "callback {} for value {} panicked",
                        entry.id, value_id
                    )),
                ),
            }
        }
    }
}

fn handle_error(handler: &ErrorHandler, error: ServerError) {
    let handler = handler.clone();
    let _ = catch_unwind(AssertUnwindSafe(|| handler(error)));
}

pub(super) fn serialize_bytes<T>(value: &T) -> Result<Bytes>
where
    T: Serialize,
{
    serialize::<T, 32>(value)
        .map(|data| data.to_bytes())
        .map_err(|_| ServerError::new("failed to serialize value"))
}

pub(super) fn deserialize_bytes<T>(data: &[u8]) -> Result<T>
where
    T: for<'a> Deserialize<'a>,
{
    let (value, used) = deserialize_value::<T>(data)
        .map_err(|_| ServerError::new("failed to deserialize value"))?;
    if used != data.len() {
        return Err(ServerError::new(
            "deserialized value did not consume all bytes",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    #[cfg(feature = "client")]
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[cfg(feature = "client")]
    struct ClientTestState {
        value: crate::Value<i32>,
    }

    #[cfg(feature = "client")]
    impl crate::State for ClientTestState {
        const NAME: &'static str = "ClientTestState";

        fn new(c: &mut impl crate::StatesCreator) -> Self {
            Self {
                value: c.value("value", 0),
            }
        }
    }

    #[test]
    fn callback_handle_unregisters_on_drop() {
        let server = StateServer::new().unwrap();
        let (id, signal) = server
            .add_signal::<u32>("root.signal".to_string(), false)
            .unwrap();

        let handle = server.add_raw_callback(id, |_, _| Ok(()));
        assert_eq!(server.inner.callbacks.get(id).len(), 1);

        drop(handle);
        assert!(server.inner.callbacks.get(id).is_empty());

        signal.set(serialize_bytes(&42_u32).unwrap());
        assert!(server.inner.signals.try_changed_value(None).is_none());
    }

    #[test]
    fn failed_start_does_not_spawn_signal_workers() {
        let server = StateServer::new().unwrap();

        assert!(server.start(0, None, None).is_err());
        assert!(server.inner.workers.lock().is_empty());
    }

    #[test]
    fn bind_errors_are_reported_and_start_can_be_retried() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = StateServer::new().unwrap();
        server.finalize().unwrap();

        let error = server
            .start(port, Some(Ipv4Addr::LOCALHOST), None)
            .unwrap_err();
        assert!(error.message().contains("binding failed"));
        assert!(!server.is_running());
        assert!(server.inner.workers.lock().is_empty());

        drop(listener);
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();
        assert!(server.is_running());
        server.stop();
    }

    #[test]
    fn running_server_stops_when_dropped() {
        if crate::test_support::isolated() {
            return;
        }
        let server = StateServer::new().unwrap();
        server.finalize().unwrap();
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();
        drop(server);
        let _released = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .expect("dropping the last server handle must release its listener");
    }

    #[test]
    fn start_is_idempotent_while_server_is_running() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let server = StateServer::new().unwrap();
        server.finalize().unwrap();
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();

        server
            .start(
                port,
                Some(Ipv4Addr::LOCALHOST),
                Some("ignored-token".to_string()),
            )
            .unwrap();
        assert!(server.is_running());
        server.stop();
    }

    #[cfg(feature = "client")]
    #[test]
    fn rust_client_and_server_exchange_values() {
        if crate::test_support::isolated() {
            return;
        }
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let server = StateServer::new().unwrap();
        let server_value = crate::server::Value::new(&server, "root.value", 0_i32, false).unwrap();
        server.finalize().unwrap();
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();

        let (client_state, client) = crate::ClientBuilder::<ClientTestState>::new().build(port);
        let mut connected = false;
        for _ in 0..200 {
            client.connect();
            if server.is_connected() {
                connected = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(connected, "Rust client did not connect to the Rust server");

        server_value.set(123, true).unwrap();
        for _ in 0..100 {
            if client_state.value.get() == 123 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(client_state.value.get(), 123);

        let (sender, receiver) = mpsc::sync_channel(1);
        let handle = server_value.connect(move |value| sender.send(value).unwrap());
        client_state.value.set_signal(456);
        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), 456);
        assert_eq!(server_value.get().unwrap(), 456);

        drop(handle);
        client.disconnect();
        server.stop();
    }

    #[cfg(feature = "client")]
    #[test]
    fn stopped_server_restarts_with_new_port_and_token() {
        if crate::test_support::isolated() {
            return;
        }
        let first_listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let second_listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let first_port = first_listener.local_addr().unwrap().port();
        let second_port = second_listener.local_addr().unwrap().port();
        drop(first_listener);
        drop(second_listener);

        let mut options = ServerOptions::new();
        options.version = Some(17);
        let server = StateServer::with_options(options).unwrap();
        let server_value = crate::server::Value::new(&server, "root.value", 7_i32, false).unwrap();
        server.finalize().unwrap();

        server
            .start(
                first_port,
                Some(Ipv4Addr::LOCALHOST),
                Some("first-token".to_string()),
            )
            .unwrap();
        let (first_state, first_client) = crate::ClientBuilder::<ClientTestState>::new()
            .version(17)
            .token("first-token".to_string())
            .build(first_port);
        for _ in 0..200 {
            first_client.connect();
            if server.is_connected() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(server.is_connected(), "first client did not connect");
        server_value.set(41, false).unwrap();
        first_client.disconnect();
        server.stop();
        drop(first_state);
        drop(first_client);

        server
            .start(
                second_port,
                Some(Ipv4Addr::LOCALHOST),
                Some("second-token".to_string()),
            )
            .unwrap();
        let (second_state, second_client) = crate::ClientBuilder::<ClientTestState>::new()
            .version(17)
            .token("second-token".to_string())
            .build(second_port);
        for _ in 0..200 {
            second_client.connect();
            if server.is_connected() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(server.is_connected(), "second client did not connect");
        for _ in 0..100 {
            if second_state.value.get() == 41 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(second_state.value.get(), 41);

        second_client.disconnect();
        server.stop();
    }

    #[test]
    fn signal_workers_dispatch_and_shutdown() {
        let mut options = ServerOptions::new();
        options.signal_workers = 3;
        let server = StateServer::with_options(options).unwrap();
        let (id, signal) = server
            .add_signal::<u32>("root.signal".to_string(), false)
            .unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let handle = server.add_typed_callback(id, move |value: u32| {
            sender.send(value).unwrap();
        });

        server.start_signal_workers();
        signal.set(serialize_bytes(&7_u32).unwrap());

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            7_u32
        );

        drop(handle);
        drop(server);
    }

    #[test]
    fn connect_previous_receives_the_replaced_value() {
        let mut options = ServerOptions::new();
        options.signal_workers = 1;
        let server = StateServer::with_options(options).unwrap();
        let value = crate::server::Value::new(&server, "root.value", 1_i32, true).unwrap();
        let (sender, receiver) = mpsc::sync_channel(4);
        let handle = value.connect_previous(move |new: i32, previous: i32| {
            sender.send((new, previous)).unwrap();
        });

        server.start_signal_workers();
        // The initial value seeds the chain, so the first change already has a previous.
        value.set_signal(2, false).unwrap();
        value.set_signal(3, false).unwrap();

        let mut seen = Vec::new();
        while seen.len() < 2 {
            match receiver.recv_timeout(Duration::from_secs(1)) {
                Ok(pair) => seen.push(pair),
                Err(_) => break,
            }
        }

        // Queue mode, so both changes are delivered and the chain is contiguous.
        assert_eq!(seen, vec![(2, 1), (3, 2)]);

        drop(handle);
        drop(server);
    }

    /// A plain `connect` must not start carrying the previous value, and a
    /// `connect_previous` alongside it must still get one.
    #[test]
    fn plain_and_previous_callbacks_coexist_on_one_value() {
        let mut options = ServerOptions::new();
        options.signal_workers = 1;
        let server = StateServer::with_options(options).unwrap();
        let value = crate::server::Value::new(&server, "root.value", 1_i32, false).unwrap();

        let (plain_sender, plain_receiver) = mpsc::sync_channel(1);
        let plain = value.connect(move |new: i32| plain_sender.send(new).unwrap());
        let (pair_sender, pair_receiver) = mpsc::sync_channel(1);
        let with_previous = value.connect_previous(move |new: i32, previous: i32| {
            pair_sender.send((new, previous)).unwrap();
        });

        server.start_signal_workers();
        value.set_signal(2, false).unwrap();

        assert_eq!(plain_receiver.recv_timeout(Duration::from_secs(1)), Ok(2));
        assert_eq!(
            pair_receiver.recv_timeout(Duration::from_secs(1)),
            Ok((2, 1))
        );

        drop(plain);
        drop(with_previous);
        drop(server);
    }

    /// `run_signal_worker` snapshots callbacks only after the claim has decided whether
    /// to carry a previous value, so a callback registered in between is handed a change
    /// without one. It must skip that change quietly instead of reporting an error: it
    /// was not connected when the change was claimed.
    #[test]
    fn a_previous_callback_skips_a_change_that_carries_no_previous() {
        let server = StateServer::new().unwrap();
        let (id, _value) = server
            .add_value("root.value".to_string(), 1_i32, false)
            .unwrap();
        let fired = Arc::new(AtomicBool::new(false));
        let flag = fired.clone();
        let handle = server.add_typed_callback_previous(id, move |_: i32, _: i32| {
            flag.store(true, Ordering::Relaxed);
        });
        let entry = server.inner.callbacks.get(id).remove(0);

        // Exactly what the worker passes for a change claimed before registration.
        assert!(
            (entry.callback)(serialize_bytes(&2_i32).unwrap(), None).is_ok(),
            "a missing previous value is not an error"
        );
        assert!(!fired.load(Ordering::Relaxed));

        // The same callback still fires for a change that does carry one.
        assert!(
            (entry.callback)(
                serialize_bytes(&2_i32).unwrap(),
                Some(serialize_bytes(&1_i32).unwrap()),
            )
            .is_ok()
        );
        assert!(fired.load(Ordering::Relaxed));

        drop(handle);
    }

    /// Dropping the only previous-value callback has to clear the registration flag,
    /// otherwise the previous value keeps being carried for nobody.
    #[test]
    fn dropping_the_last_previous_callback_stops_carrying_the_previous_value() {
        let server = StateServer::new().unwrap();
        let value = crate::server::Value::new(&server, "root.value", 1_i32, false).unwrap();

        let plain = value.connect(|_: i32| {});
        let with_previous = value.connect_previous(|_: i32, _: i32| {});
        value.set_signal(2, false).unwrap();
        let (id, _, previous) = server.inner.signals.try_changed_value(None).unwrap();
        assert_eq!(previous, Some(serialize_bytes(&1_i32).unwrap()));

        drop(with_previous);
        value.set_signal(3, false).unwrap();
        // `id` has to come back as `last_id`, otherwise it stays blocked and is skipped.
        let (_, _, previous) = server.inner.signals.try_changed_value(Some(id)).unwrap();
        assert_eq!(
            previous, None,
            "the previous value must not be delivered once nobody asks for it"
        );

        drop(plain);
    }

    #[test]
    fn drop_does_not_block_on_a_worker_stuck_in_a_callback() {
        if crate::test_support::isolated() {
            return;
        }
        let (release_sender, release_receiver) = mpsc::channel::<()>();
        let (entered_sender, entered_receiver) = mpsc::channel::<()>();

        let mut options = ServerOptions::new();
        options.signal_workers = 1;
        options.shutdown_timeout = Duration::from_millis(200);
        let server = StateServer::with_options(options).unwrap();
        let (id, signal) = server
            .add_signal::<u32>("root.signal".to_string(), false)
            .unwrap();

        let release_receiver = Mutex::new(release_receiver);
        let handle = server.add_raw_callback(id, move |_, _| {
            entered_sender.send(()).unwrap();
            // Stands in for any callback that blocks: a socket, a mutex, or a
            // `ValueTake::set(.., blocking = true, ..)` awaiting a client ack.
            let _ = release_receiver.lock().recv();
            Ok(())
        });

        server.start_signal_workers();
        signal.set(serialize_bytes(&1_u32).unwrap());
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("callback should have been entered");

        // The worker cannot observe the shutdown flag until its callback
        // returns, so `drop` has to fall back to detaching it. Dropped on
        // another thread so a regression fails the test instead of hanging it.
        let (dropped_sender, dropped_receiver) = mpsc::channel::<()>();
        thread::spawn(move || {
            drop(handle);
            drop(signal);
            drop(server);
            let _ = dropped_sender.send(());
        });

        let dropped = dropped_receiver
            .recv_timeout(Duration::from_secs(5))
            .is_ok();
        // Let the callback return either way, so the detached worker exits.
        let _ = release_sender.send(());
        assert!(dropped, "drop blocked on a worker stuck in a callback");
    }

    #[test]
    fn drop_is_prompt_when_workers_are_idle() {
        if crate::test_support::isolated() {
            return;
        }
        let mut options = ServerOptions::new();
        options.signal_workers = 3;
        options.shutdown_timeout = Duration::from_secs(5);
        let server = StateServer::with_options(options).unwrap();
        server.start_signal_workers();

        // Idle workers observe the shutdown flag immediately, so teardown must
        // not wait out the timeout.
        let start = std::time::Instant::now();
        drop(server);

        assert!(
            start.elapsed() < Duration::from_secs(1),
            "idle workers should exit without waiting out the shutdown timeout"
        );
    }

    #[test]
    fn deserialize_rejects_trailing_bytes() {
        let mut data = serialize_bytes(&12_u16).unwrap().to_vec();
        data.push(0);

        assert_eq!(
            deserialize_bytes::<u16>(&data).unwrap_err().message(),
            "deserialized value did not consume all bytes"
        );
    }
}

#[cfg(all(test, feature = "client"))]
mod wire_tests {
    use super::*;
    use crate::serialization::{ClientHeader, ServerHeader};
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    async fn peer(port: u16, header: ClientHeader, valid: bool) {
        let stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        let (mut ws, _) = tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}"), stream)
            .await
            .unwrap();
        ws.send(Message::Binary(
            postcard::to_stdvec(&header).unwrap().into(),
        ))
        .await
        .unwrap();
        let response = ws.next().await;
        if valid {
            let Some(Ok(Message::Binary(bytes))) = response else {
                panic!("valid handshake did not synchronize");
            };
            let (header, offset) = ServerHeader::deserialize(&bytes).unwrap();
            match header {
                ServerHeader::Value(_, _, false, length) => {
                    assert_eq!(
                        postcard::from_bytes::<i32>(&bytes[offset..offset + length as usize])
                            .unwrap(),
                        731
                    );
                }
                _ => panic!("non-default state missing from sync"),
            }
            ws.close(None).await.unwrap();
        } else {
            assert!(
                !matches!(response, Some(Ok(Message::Binary(_)))),
                "rejected client received synchronization"
            );
        }
    }

    #[test]
    fn rejected_handshakes_recover_and_start_restart_preserve_credentials() {
        if crate::test_support::isolated() {
            return;
        }
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let server = StateServer::with_options(ServerOptions {
            version: Some(17),
            ..Default::default()
        })
        .unwrap();
        let _value = crate::server::Value::new(&server, "root.value", 731i32, false).unwrap();
        server.finalize().unwrap();
        server
            .start(port, Some(Ipv4Addr::LOCALHOST), Some("first".into()))
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        let good =
            || ClientHeader::Handshake(crate::PROTOCOL_VERSION, Some(17), Some("first".into()));
        runtime.block_on(async {
            for bad in [
                ClientHeader::Ack(1),
                ClientHeader::Handshake(0, Some(17), Some("first".into())),
                ClientHeader::Handshake(
                    crate::PROTOCOL_VERSION + 1,
                    Some(17),
                    Some("first".into()),
                ),
                ClientHeader::Handshake(crate::PROTOCOL_VERSION, None, Some("first".into())),
                ClientHeader::Handshake(crate::PROTOCOL_VERSION, Some(18), Some("first".into())),
                ClientHeader::Handshake(crate::PROTOCOL_VERSION, Some(17), None),
                ClientHeader::Handshake(crate::PROTOCOL_VERSION, Some(17), Some("wrong".into())),
            ] {
                peer(port, bad, false).await;
                peer(port, good(), true).await;
            }
            // A repeated start cannot change the original address or token.
            let occupied = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            server
                .start(
                    occupied.local_addr().unwrap().port(),
                    Some(Ipv4Addr::LOCALHOST),
                    Some("ignored".into()),
                )
                .unwrap();
            peer(
                port,
                ClientHeader::Handshake(crate::PROTOCOL_VERSION, Some(17), Some("ignored".into())),
                false,
            )
            .await;
            peer(port, good(), true).await;
        });
        server.stop();
        server
            .start(port, Some(Ipv4Addr::LOCALHOST), Some("second".into()))
            .unwrap();
        runtime.block_on(async {
            peer(port, good(), false).await;
            peer(
                port,
                ClientHeader::Handshake(crate::PROTOCOL_VERSION, Some(17), Some("second".into())),
                true,
            )
            .await;
        });
        server.stop();
    }

    #[test]
    fn connection_message_and_disconnection_callbacks_follow_socket_events() {
        if crate::test_support::isolated() {
            return;
        }
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let server = StateServer::new().unwrap();
        server.finalize().unwrap();
        let (tx, rx) = mpsc::channel();
        let connect = tx.clone();
        let _on_connect = server.on_connect(move |address| {
            assert!(address.starts_with("127.0.0.1:"));
            connect.send("connect".to_string()).unwrap();
        });
        let message = tx.clone();
        let _on_message = server.on_client_message(move |value| {
            message.send(value).unwrap();
        });
        let _on_disconnect = server.on_disconnect(move || {
            tx.send("disconnect".into()).unwrap();
        });
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        runtime.block_on(async {
            let stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            let (mut ws, _) =
                tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}"), stream)
                    .await
                    .unwrap();
            ws.send(Message::Binary(
                postcard::to_stdvec(&ClientHeader::Handshake(
                    crate::PROTOCOL_VERSION,
                    None,
                    None,
                ))
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
            assert!(matches!(ws.next().await, Some(Ok(Message::Binary(_)))));
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "connect");
            let payload = postcard::to_stdvec("client diagnostic 🦀").unwrap();
            let mut bytes =
                postcard::to_stdvec(&ClientHeader::Message(payload.len() as u32)).unwrap();
            bytes.extend(payload);
            ws.send(Message::Binary(bytes.into())).await.unwrap();
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                "client diagnostic 🦀"
            );
            ws.close(None).await.unwrap();
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "disconnect"
        );
        assert!(rx.try_recv().is_err());
        server.stop();
    }
}
