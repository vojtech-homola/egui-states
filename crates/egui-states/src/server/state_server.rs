//! Native server lifecycle and callback-worker management.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
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
use crate::server_core::web::WebSite;

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

        let workers = std::mem::take(self.workers.get_mut());
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

    /// Serves the static web page in directory `path` on the server port.
    ///
    /// This lets a browser load a WASM client from the same port it then
    /// connects to. WebSocket upgrade requests reach the state server as before;
    /// other HTTP `GET` and `HEAD` requests are answered from the page, with
    /// `/` and paths ending in `/` mapped to `index.html`.
    ///
    /// Every file below `path` is read into memory during this call, so later
    /// changes on disk are not served until it is called again. Calling it
    /// again replaces the previous page, also with a different directory. The
    /// page may be set before [`Self::start`], applies to connections accepted
    /// afterwards, and stays configured across [`Self::stop`] and restarts. A
    /// page load that overlaps a replacement may combine files of both pages;
    /// reloading the page resolves it.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` is not a directory, a file cannot be read,
    /// or the directory does not contain `index.html`. The previously served
    /// page, if any, is kept in that case.
    pub fn serve_web(&self, path: impl AsRef<Path>) -> Result<()> {
        let site = WebSite::load(path.as_ref()).map_err(ServerError::new)?;
        self.inner.server.read().set_web(Some(site));
        Ok(())
    }

    /// Stops serving the static web page.
    ///
    /// Without a page, the port accepts only WebSocket connections, as if
    /// [`Self::serve_web`] had never been called. Requests already being
    /// answered complete; the connected client is not affected.
    pub fn stop_web(&self) {
        self.inner.server.read().set_web(None);
    }

    /// Returns the absolute directory of the served web page, if any.
    pub fn web_path(&self) -> Option<PathBuf> {
        self.inner.server.read().web_path()
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
        .map(|data| data.into_bytes())
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
    fn unfinalized_start_fails_without_spawning_signal_workers() {
        let server = StateServer::new().unwrap();

        let error = server.start(0, None, None).unwrap_err();
        assert!(error.message().contains("not been finalized"), "{error}");
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
    fn signal_workers_dispatch_callbacks() {
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

    type Register = fn(&StateServer, &str) -> Result<()>;

    /// One registration per state kind, all sharing the server's id namespace.
    fn registrations() -> [(&'static str, Register); 12] {
        use crate::server as s;
        [
            ("Value", |sv, n| {
                s::Value::new(sv, n, 0_i32, false).map(drop)
            }),
            ("Static", |sv, n| s::Static::new(sv, n, 0_i32).map(drop)),
            ("ValueTake", |sv, n| {
                s::ValueTake::<i32>::new(sv, n).map(drop)
            }),
            ("Signal", |sv, n| {
                s::Signal::<i32>::new(sv, n, false).map(drop)
            }),
            ("VecState", |sv, n| s::VecState::<i32>::new(sv, n).map(drop)),
            ("MapState", |sv, n| {
                s::MapState::<u8, i32>::new(sv, n).map(drop)
            }),
            ("Image", |sv, n| s::Image::new(sv, n).map(drop)),
            ("ImageMulti", |sv, n| s::ImageMulti::new(sv, n).map(drop)),
            ("Data", |sv, n| s::Data::<u8>::new(sv, n).map(drop)),
            ("DataMulti", |sv, n| {
                s::DataMulti::<u8>::new(sv, n).map(drop)
            }),
            ("DataTake", |sv, n| s::DataTake::<u8>::new(sv, n).map(drop)),
            ("DataMultiTake", |sv, n| {
                s::DataMultiTake::<u8>::new(sv, n).map(drop)
            }),
        ]
    }

    #[test]
    fn every_state_kind_is_rejected_after_finalize() {
        for (kind, register) in registrations() {
            let server = StateServer::new().unwrap();
            server.finalize().unwrap();
            let error = register(&server, "root.late").expect_err(kind);
            assert!(error.message().contains("finalized"), "{kind}: {error}");
        }
    }

    #[test]
    fn every_state_kind_rejects_a_name_used_by_any_kind() {
        for (first_kind, first) in registrations() {
            for (second_kind, second) in registrations() {
                let server = StateServer::new().unwrap();
                first(&server, "root.same").unwrap();
                let error = second(&server, "root.same")
                    .expect_err(&format!("{second_kind} reused the name of a {first_kind}"));
                assert!(error.message().contains("already exists"), "{error}");
            }
        }
    }

    /// Reserved ids 0-9 belong to the server's own events.
    #[test]
    fn a_state_never_receives_a_reserved_id() {
        let server = StateServer::new().unwrap();
        for index in 0..64 {
            let (id, _) = server
                .add_signal::<u32>(format!("root.signal_{index}"), false)
                .unwrap();
            assert!(id > 9, "root.signal_{index} got reserved id {id}");
        }
    }

    #[test]
    fn panicking_callbacks_and_error_handlers_do_not_stop_the_worker() {
        let handled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = handled.clone();
        let mut options = ServerOptions::new();
        options.signal_workers = 1;
        options.error_handler = Some(Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            panic!("error handler failed");
        }));
        let server = StateServer::with_options(options).unwrap();
        let (id, signal) = server
            .add_signal::<u32>("root.signal".to_string(), true)
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let _handle = server.add_typed_callback(id, move |value: u32| {
            sender.send(value).unwrap();
            if value < 3 {
                panic!("callback failed");
            }
        });

        server.start_signal_workers();
        for value in 1..=3_u32 {
            signal.set(serialize_bytes(&value).unwrap());
        }
        let received: Vec<_> = (0..3)
            .map(|_| receiver.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(received, [1, 2, 3]);
        assert_eq!(handled.load(Ordering::SeqCst), 2);
    }

    /// Callbacks of one state run one at a time and in order, even when other
    /// workers are idle.
    #[test]
    fn callbacks_for_one_state_never_overlap_and_stay_ordered() {
        let mut options = ServerOptions::new();
        options.signal_workers = 3;
        let server = StateServer::with_options(options).unwrap();
        let value = crate::server::Value::new(&server, "root.value", 0_u32, true).unwrap();
        let running = Arc::new(AtomicBool::new(false));
        let overlapped = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (flag, overlap, record) = (running.clone(), overlapped.clone(), seen.clone());
        let _handle = value.connect(move |new: u32| {
            if flag.swap(true, Ordering::SeqCst) {
                overlap.store(true, Ordering::SeqCst);
            }
            thread::sleep(Duration::from_millis(1));
            record.lock().push(new);
            flag.store(false, Ordering::SeqCst);
        });

        server.start_signal_workers();
        for new in 1..=40 {
            value.set_signal(new, false).unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while seen.lock().len() < 40 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!overlapped.load(Ordering::SeqCst), "callbacks overlapped");
        assert_eq!(*seen.lock(), (1..=40).collect::<Vec<_>>());
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

#[cfg(all(test, feature = "client"))]
mod connection_tests {
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::serialization::{ClientHeader, ServerHeader, VALUE_MAX_SIZE};
    use crate::{ClientBuilder, ConnectionState};
    use tokio_tungstenite::tungstenite::{self, Message};

    struct ClientTestState {
        value: crate::Value<i32>,
    }

    impl crate::State for ClientTestState {
        const NAME: &'static str = "ClientTestState";

        fn new(c: &mut impl crate::StatesCreator) -> Self {
            Self {
                value: c.value("value", 0),
            }
        }
    }

    struct CollectionTestState {
        items: crate::VecState<String>,
        entries: crate::MapState<u32, String>,
    }

    impl crate::State for CollectionTestState {
        const NAME: &'static str = "CollectionTestState";

        fn new(c: &mut impl crate::StatesCreator) -> Self {
            Self {
                items: c.vec("items"),
                entries: c.map("entries"),
            }
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn wait(label: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out: {label}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// A server with one `i32` value, started on a free port.
    fn server(
        options: ServerOptions,
        token: Option<&str>,
    ) -> (StateServer, crate::server::Value<i32>, u16) {
        let port = free_port();
        let server = StateServer::with_options(options).unwrap();
        let value = crate::server::Value::new(&server, "root.value", 731_i32, false).unwrap();
        server.finalize().unwrap();
        server
            .start(port, Some(Ipv4Addr::LOCALHOST), token.map(str::to_owned))
            .unwrap();
        (server, value, port)
    }

    enum Event {
        Frame(Vec<(ServerHeader, Vec<u8>)>),
        Closed,
        Silent,
    }

    /// Splits a server frame into its messages and their payloads.
    fn messages(bytes: &[u8]) -> Vec<(ServerHeader, Vec<u8>)> {
        let mut rest = bytes;
        let mut messages = Vec::new();
        while !rest.is_empty() {
            let (header, size) = ServerHeader::deserialize(rest).unwrap();
            let payload = match &header {
                ServerHeader::Value(.., length) | ServerHeader::Static(.., length) => {
                    *length as usize
                }
                ServerHeader::ValueVec(.., length) => *length as usize,
                ServerHeader::Update(_) => 0,
                _ => panic!("unexpected message kind in a connection test"),
            };
            messages.push((header, rest[size..size + payload].to_vec()));
            rest = &rest[size + payload..];
        }
        messages
    }

    /// A raw synchronous WebSocket peer speaking the client protocol.
    struct Peer(tungstenite::WebSocket<TcpStream>);

    impl Peer {
        fn connect(port: u16, token: Option<&str>) -> Self {
            let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            let (socket, _) =
                tungstenite::client(format!("ws://127.0.0.1:{port}"), stream).unwrap();
            socket
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            let mut peer = Self(socket);
            let handshake =
                ClientHeader::Handshake(crate::PROTOCOL_VERSION, None, token.map(str::to_owned));
            peer.send(handshake, &[]);
            peer
        }

        fn send(&mut self, header: ClientHeader, payload: &[u8]) {
            let mut bytes = postcard::to_stdvec(&header).unwrap();
            bytes.extend(payload);
            self.0.send(Message::Binary(bytes.into())).unwrap();
        }

        fn next(&mut self, within: Duration) -> Event {
            let deadline = Instant::now() + within;
            loop {
                match self.0.read() {
                    Ok(Message::Binary(bytes)) => return Event::Frame(messages(&bytes)),
                    Ok(Message::Close(_)) => return Event::Closed,
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(error))
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        if Instant::now() >= deadline {
                            return Event::Silent;
                        }
                    }
                    Err(_) => return Event::Closed,
                }
            }
        }

        /// Reads frames until one holds a message matching `found`.
        fn wait_for(&mut self, label: &str, mut found: impl FnMut(&ServerHeader, &[u8]) -> bool) {
            loop {
                match self.next(Duration::from_secs(5)) {
                    Event::Frame(messages) => {
                        if messages
                            .iter()
                            .any(|(header, payload)| found(header, payload))
                        {
                            return;
                        }
                    }
                    Event::Closed => panic!("{label}: connection closed"),
                    Event::Silent => panic!("{label}: no message within 5s"),
                }
            }
        }

        fn wait_for_value(&mut self, expected: i32) {
            self.wait_for(&format!("value {expected}"), |header, payload| {
                matches!(header, ServerHeader::Value(..))
                    && postcard::from_bytes::<i32>(payload).unwrap() == expected
            });
        }

        fn close(mut self) {
            let _ = self.0.close(None);
            while !matches!(
                self.next(Duration::from_secs(5)),
                Event::Closed | Event::Silent
            ) {}
        }
    }

    #[test]
    fn a_second_client_takes_over_the_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, value, port) = server(ServerOptions::new(), None);
        let mut first = Peer::connect(port, None);
        first.wait_for_value(731);

        let mut second = Peer::connect(port, None);
        second.wait_for_value(731);
        assert!(
            matches!(first.next(Duration::from_secs(5)), Event::Closed),
            "the replaced client stayed connected"
        );

        value.set(5, false).unwrap();
        second.wait_for_value(5);
        assert!(server.is_connected());
        server.stop();
    }

    #[test]
    fn a_rejected_client_does_not_disturb_the_connected_one() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, value, port) = server(ServerOptions::new(), Some("secret"));
        let mut accepted = Peer::connect(port, Some("secret"));
        accepted.wait_for_value(731);

        let mut rejected = Peer::connect(port, Some("wrong"));
        assert!(
            matches!(rejected.next(Duration::from_secs(5)), Event::Closed),
            "a rejected client must be closed without data"
        );

        value.set(6, false).unwrap();
        accepted.wait_for_value(6);
        assert!(server.is_connected());
        server.stop();
    }

    #[test]
    fn repaint_requests_reach_the_client() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, _value, port) = server(ServerOptions::new(), None);
        let mut peer = Peer::connect(port, None);
        // Every synchronization ends with an immediate repaint.
        peer.wait_for(
            "initial repaint",
            |header, _| matches!(header, ServerHeader::Update(delay) if *delay == 0.0),
        );

        server.update(Some(0.5)).unwrap();
        peer.wait_for(
            "delayed repaint",
            |header, _| matches!(header, ServerHeader::Update(delay) if *delay == 0.5),
        );
        server.update(None).unwrap();
        peer.wait_for(
            "immediate repaint",
            |header, _| matches!(header, ServerHeader::Update(delay) if *delay == 0.0),
        );
        server.stop();
    }

    #[test]
    fn disconnect_client_keeps_the_server_listening() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, _value, port) = server(ServerOptions::new(), None);
        let mut first = Peer::connect(port, None);
        first.wait_for_value(731);

        server.disconnect_client();
        assert!(matches!(first.next(Duration::from_secs(5)), Event::Closed));
        wait("disconnect", || !server.is_connected());
        assert!(server.is_running());

        let mut second = Peer::connect(port, None);
        second.wait_for_value(731);
        server.stop();
    }

    #[test]
    fn rejected_handshakes_are_logged_as_warnings_only() {
        if crate::test_support::isolated() {
            return;
        }
        let port = free_port();
        let server = StateServer::new().unwrap();
        server.finalize().unwrap();
        let logging = crate::server::LoggingSignal::new(&server);
        let (warnings, warning) = mpsc::channel();
        let (errors, error) = mpsc::channel();
        let _warnings = logging.add_logger(crate::server::LogLevel::Warning, move |message| {
            let _ = warnings.send(message);
        });
        let _errors = logging.add_logger(crate::server::LogLevel::Error, move |message| {
            let _ = errors.send(message);
        });
        server
            .start(port, Some(Ipv4Addr::LOCALHOST), Some("secret".into()))
            .unwrap();

        let mut rejected = Peer::connect(port, Some("wrong"));
        assert!(matches!(
            rejected.next(Duration::from_secs(5)),
            Event::Closed
        ));
        let message = warning.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(message.contains("wrong token"), "{message}");
        assert!(
            error.recv_timeout(Duration::from_millis(200)).is_err(),
            "a rejected handshake is not a server error"
        );
        server.stop();
    }

    /// A disconnect resets pending signals while a worker may still be running a
    /// callback; the next connection must not start the same state's callback on
    /// a second worker before that one returns.
    #[test]
    fn callbacks_for_one_state_do_not_overlap_across_a_reconnect() {
        if crate::test_support::isolated() {
            return;
        }
        let port = free_port();
        let server = StateServer::new().unwrap();
        let signal = crate::server::Signal::<i32>::new(&server, "root.signal", true).unwrap();
        server.finalize().unwrap();
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();

        let (entered_sender, entered) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let running = Arc::new(AtomicBool::new(false));
        let overlapped = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (flag, overlap, record) = (running.clone(), overlapped.clone(), seen.clone());
        let _handle = signal.connect(move |value: i32| {
            if flag.swap(true, Ordering::SeqCst) {
                overlap.store(true, Ordering::SeqCst);
            }
            record.lock().push(value);
            if value == 1 {
                entered_sender.send(()).unwrap();
                let _ = released.lock().recv_timeout(Duration::from_secs(5));
            }
            flag.store(false, Ordering::SeqCst);
        });

        let id = crate::hashing::generate_value_id("root.signal");
        let type_id = <i32 as Typed>::get_type().get_hash();
        let send = |peer: &mut Peer, value: i32| {
            let payload = postcard::to_stdvec(&value).unwrap();
            peer.send(
                ClientHeader::Signal(id, type_id, payload.len() as u32),
                &payload,
            );
        };
        let initial_repaint =
            |header: &ServerHeader, _: &[u8]| matches!(header, ServerHeader::Update(_));

        let mut first = Peer::connect(port, None);
        first.wait_for("first connection", initial_repaint);
        send(&mut first, 1);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        first.close();
        wait("first disconnect", || !server.is_connected());
        thread::sleep(Duration::from_millis(50));

        let mut second = Peer::connect(port, None);
        second.wait_for("second connection", initial_repaint);
        send(&mut second, 2);
        thread::sleep(Duration::from_millis(300));
        let early = seen.lock().clone();
        let overlapped_early = overlapped.load(Ordering::SeqCst);
        let _ = release.send(());

        wait("second callback", || seen.lock().len() == 2);
        assert!(
            !overlapped_early && early == [1],
            "the second callback started while the first was still running: {early:?}"
        );
        assert!(!overlapped.load(Ordering::SeqCst));
        assert_eq!(*seen.lock(), [1, 2]);
        server.stop();
    }

    /// `Connected` means the server accepted the handshake, not merely that the
    /// handshake was sent. This peer reads the handshake and never answers.
    #[test]
    fn client_reports_connected_only_after_the_server_accepts() {
        if crate::test_support::isolated() {
            return;
        }
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(AtomicBool::new(false));
        let handshake_received = received.clone();
        let silent_server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let _handshake = socket.read();
            handshake_received.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_secs(1));
        });

        let (_state, client) = ClientBuilder::<ClientTestState>::new().build(port);
        let mut connected = false;
        // Repeat the request until the handshake arrives, then keep watching.
        let mut deadline = Instant::now() + Duration::from_secs(5);
        let mut watching = false;
        while Instant::now() < deadline && !connected {
            if !watching {
                client.connect();
                if received.load(Ordering::SeqCst) {
                    watching = true;
                    deadline = Instant::now() + Duration::from_millis(500);
                }
            }
            connected |= client.get_state() == ConnectionState::Connected;
            thread::sleep(Duration::from_millis(1));
        }
        client.disconnect();
        silent_server.join().unwrap();
        assert!(watching, "the client never sent its handshake");
        assert!(
            !connected,
            "client reported Connected before the server accepted it"
        );
    }

    /// An unreachable server and a refused handshake end in `Failed` with the
    /// cause.
    #[test]
    fn failed_connection_attempts_report_the_cause() {
        if crate::test_support::isolated() {
            return;
        }
        let (_state, client) = ClientBuilder::<ClientTestState>::new().build(free_port());
        assert_eq!(client.last_error(), None);
        client.connect();
        wait("unreachable server", || {
            client.get_state() == ConnectionState::Failed
        });
        let error = client.last_error().unwrap();
        assert!(error.contains("Connecting to server"), "{error}");

        let (server, _value, port) = server(ServerOptions::new(), Some("secret"));
        let (_state, client) = ClientBuilder::<ClientTestState>::new()
            .token("wrong".to_string())
            .build(port);
        client.connect();
        wait("refused handshake", || {
            client.get_state() == ConnectionState::Failed
        });
        let error = client.last_error().unwrap();
        assert!(
            error.contains("before the server accepted the handshake"),
            "{error}"
        );
        server.stop();
    }

    const PANIC_VALUE: i32 = -13;

    /// An `i32` whose client-side decoding panics on [`PANIC_VALUE`], standing
    /// in for a panicking user `Deserialize` implementation.
    #[derive(Clone, Serialize)]
    struct PanicOnDecode(i32);

    impl<'de> Deserialize<'de> for PanicOnDecode {
        fn deserialize<D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            let value = i32::deserialize(deserializer)?;
            if value == PANIC_VALUE {
                panic!("injected receive task panic");
            }
            Ok(Self(value))
        }
    }

    // SAFETY: serialized exactly like the wrapped `i32`.
    unsafe impl Typed for PanicOnDecode {
        fn get_type() -> crate::ObjectType {
            <i32 as Typed>::get_type()
        }
    }

    // SAFETY: same shape and meaning as the wrapped `i32`.
    unsafe impl crate::InitialValue for PanicOnDecode {
        fn init_value(&self) -> crate::InitValue {
            crate::InitialValue::init_value(&self.0)
        }
    }

    /// Like [`ClientTestState`], with a value that can panic the receive task.
    struct PanicTestState {
        value: crate::Value<PanicOnDecode>,
    }

    impl crate::State for PanicTestState {
        const NAME: &'static str = "PanicTestState";

        fn new(c: &mut impl crate::StatesCreator) -> Self {
            Self {
                value: c.value("value", PanicOnDecode(0)),
            }
        }
    }

    /// A panic while applying a server message ends the connection in `Failed`
    /// instead of stopping the client, and the next connection works in both
    /// directions.
    #[test]
    fn a_receive_task_panic_fails_the_connection_and_the_client_recovers() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, value, port) = server(ServerOptions::new(), None);
        let (state, client) = ClientBuilder::<PanicTestState>::new().build(port);
        client.connect();
        wait("first connection", || state.value.get().0 == 731);

        value.set(PANIC_VALUE, false).unwrap();
        wait("receive panic", || {
            client.get_state() == ConnectionState::Failed
        });
        let error = client.last_error().unwrap();
        assert!(
            error.contains("receive task panicked: injected receive task panic"),
            "{error}"
        );
        wait("server disconnect", || !server.is_connected());

        value.set(5, false).unwrap();
        wait("reconnection", || {
            client.connect();
            state.value.get().0 == 5
        });
        assert_eq!(client.get_state(), ConnectionState::Connected);
        state.value.set_signal(PanicOnDecode(9));
        wait("client write", || value.get().unwrap() == 9);
        client.disconnect();
        server.stop();
    }

    /// A panic in the send task ends the connection in `Failed`, closes the
    /// socket, and keeps the outgoing queue for the next connection.
    #[test]
    fn a_send_task_panic_fails_the_connection_and_the_client_recovers() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, value, port) = server(ServerOptions::new(), None);
        let (state, client) = ClientBuilder::<ClientTestState>::new().build(port);
        client.connect();
        wait("first connection", || state.value.get() == 731);

        client.inject_send_panic();
        wait("send panic", || {
            client.get_state() == ConnectionState::Failed
        });
        let error = client.last_error().unwrap();
        assert!(
            error.contains("send task panicked: injected send task panic"),
            "{error}"
        );
        wait("server disconnect", || !server.is_connected());

        value.set(5, false).unwrap();
        wait("reconnection", || {
            client.connect();
            state.value.get() == 5
        });
        assert_eq!(client.get_state(), ConnectionState::Connected);
        state.value.set_signal(9);
        wait("client write", || value.get().unwrap() == 9);
        client.disconnect();
        server.stop();
    }

    /// Repaint delays come from server code unchecked; one that does not fit
    /// in a `Duration` must not panic the client.
    #[test]
    fn an_unrepresentable_repaint_delay_keeps_the_client_connected() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, value, port) = server(ServerOptions::new(), None);
        let (state, client) = ClientBuilder::<ClientTestState>::new()
            .context(egui::Context::default())
            .build(port);
        client.connect();
        wait("first connection", || state.value.get() == 731);

        server.update(Some(f32::INFINITY)).unwrap();
        server.update(Some(1e20)).unwrap();
        value.set(5, false).unwrap();
        wait("update after the delays", || state.value.get() == 5);
        assert_eq!(client.get_state(), ConnectionState::Connected);
        client.disconnect();
        server.stop();
    }

    /// A connection request made before the background task starts waiting
    /// must not be lost.
    #[test]
    fn connect_requested_right_after_build_is_not_lost() {
        if crate::test_support::isolated() {
            return;
        }
        let (server, _value, port) = server(ServerOptions::new(), None);
        let (state, client) = ClientBuilder::<ClientTestState>::new().build(port);
        client.connect();
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.value.get() != 731 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            state.value.get(),
            731,
            "the single connect request was lost"
        );
        client.disconnect();
        server.stop();
    }

    /// Pins current behavior: writes made while disconnected are discarded, and
    /// the reconnect synchronization restores the server's value.
    #[test]
    fn writes_made_while_disconnected_are_replaced_by_the_server_value() {
        if crate::test_support::isolated() {
            return;
        }
        let port = free_port();
        let server = StateServer::new().unwrap();
        let value = crate::server::Value::new(&server, "root.value", 7_i32, false).unwrap();
        server.finalize().unwrap();
        let (sender, received) = mpsc::channel();
        let _handle = value.connect(move |new| {
            let _ = sender.send(new);
        });
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();

        let (state, client) = ClientBuilder::<ClientTestState>::new().build(port);
        wait("first connection", || {
            client.connect();
            server.is_connected() && state.value.get() == 7
        });
        server.stop();
        wait("client disconnect", || {
            client.get_state() != ConnectionState::Connected
        });

        state.value.set_signal(99);
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();
        wait("reconnection", || {
            client.connect();
            server.is_connected() && state.value.get() == 7
        });
        thread::sleep(Duration::from_millis(200));
        assert!(
            received.try_recv().is_err(),
            "an offline write reached the server"
        );
        assert_eq!(value.get().unwrap(), 7);
        client.disconnect();
        server.stop();
    }

    /// Items are limited to `VALUE_MAX_SIZE` each, but a full replacement is one
    /// message; a large collection must still reach the client.
    #[test]
    fn a_collection_larger_than_one_frame_reaches_the_client() {
        if crate::test_support::isolated() {
            return;
        }
        let port = free_port();
        let server = StateServer::new().unwrap();
        let items = crate::server::VecState::<String>::new(&server, "root.items").unwrap();
        server.finalize().unwrap();
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();

        let (state, client) = ClientBuilder::<CollectionTestState>::new().build(port);
        wait("connection", || {
            client.connect();
            server.is_connected()
        });
        items
            .set(vec!["x".repeat(VALUE_MAX_SIZE - 8); 13], false)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.items.read(Vec::len) != 13 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            state.items.read(Vec::len),
            13,
            "client state: {:?}",
            client.get_state() == ConnectionState::Connected
        );
        client.disconnect();
        server.stop();
    }

    /// Like [`a_collection_larger_than_one_frame_reaches_the_client`], for a map
    /// that is already set when the client connects, so the initial sync is split.
    #[test]
    fn a_map_larger_than_one_frame_is_synced_to_the_client() {
        if crate::test_support::isolated() {
            return;
        }
        let port = free_port();
        let server = StateServer::new().unwrap();
        let entries = crate::server::MapState::<u32, String>::new(&server, "root.entries").unwrap();
        server.finalize().unwrap();
        entries
            .set(
                (0..13)
                    .map(|key| (key, "x".repeat(VALUE_MAX_SIZE - 16)))
                    .collect(),
                false,
            )
            .unwrap();
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();

        let (state, client) = ClientBuilder::<CollectionTestState>::new().build(port);
        wait("connection", || {
            client.connect();
            server.is_connected()
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.entries.read(|map| map.len()) != 13 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(state.entries.read(|map| map.len()), 13);
        assert!(
            state
                .entries
                .read(|map| map.values().all(|v| v.len() == VALUE_MAX_SIZE - 16))
        );
        client.disconnect();
        server.stop();
    }

    /// A temporary web page directory, removed on drop.
    struct WebDir(std::path::PathBuf);

    impl WebDir {
        fn new(index: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "egui-states-page-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("index.html"), index).unwrap();
            std::fs::write(path.join("app.wasm"), b"\0asm").unwrap();
            Self(path)
        }
    }

    impl Drop for WebDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Sends raw request bytes, possibly in pieces, and reads until the server closes.
    fn http(port: u16, parts: &[&[u8]]) -> String {
        use std::io::{Read, Write};
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                thread::sleep(Duration::from_millis(50));
            }
            stream.write_all(part).unwrap();
        }
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);
        String::from_utf8_lossy(&response).into_owned()
    }

    fn get(port: u16, path: &str) -> String {
        http(
            port,
            &[format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()],
        )
    }

    #[test]
    fn web_page_and_client_share_the_port_and_switch_at_runtime() {
        if crate::test_support::isolated() {
            return;
        }
        let first = WebDir::new("first");
        let second = WebDir::new("second");
        let (server, value, port) = server(ServerOptions::new(), None);
        assert_eq!(server.web_path(), None);

        let mut peer = Peer::connect(port, None);
        peer.wait_for_value(731);

        // Enabled while running and while a client is connected.
        server.serve_web(&first.0).unwrap();
        assert_eq!(server.web_path().as_deref(), Some(first.0.as_path()));
        let page = get(port, "/");
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
        assert!(page.ends_with("\r\n\r\nfirst"), "{page}");
        let wasm = get(port, "/app.wasm");
        assert!(
            wasm.contains("Content-Type: application/wasm\r\n"),
            "{wasm}"
        );
        assert!(get(port, "/missing.js").starts_with("HTTP/1.1 404"));
        let head = http(port, &[b"HEAD / HTTP/1.1\r\n\r\n"]);
        assert!(
            head.starts_with("HTTP/1.1 200") && head.ends_with("\r\n\r\n"),
            "{head}"
        );
        let post = http(port, &[b"POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi"]);
        assert!(post.starts_with("HTTP/1.1 405"), "{post}");
        let garbage = http(port, &[b"\x00\x01 not http\r\n\r\n"]);
        assert!(garbage.starts_with("HTTP/1.1 400"), "{garbage}");

        // A failed reload keeps the current page.
        assert!(server.serve_web(first.0.join("missing")).is_err());
        assert!(get(port, "/").ends_with("first"));

        server.serve_web(&second.0).unwrap();
        assert!(get(port, "/").ends_with("second"));

        // Without a page, HTTP requests fail the WebSocket handshake as before.
        server.stop_web();
        assert_eq!(server.web_path(), None);
        assert!(!get(port, "/").starts_with("HTTP/1.1 200"));

        // None of the above disturbed the connected client.
        value.set(5, false).unwrap();
        peer.wait_for_value(5);
        assert!(server.is_connected());
        server.stop();
    }

    #[test]
    fn idle_connections_do_not_block_new_clients() {
        if crate::test_support::isolated() {
            return;
        }
        let page = WebDir::new("page");
        for serve_page in [false, true] {
            let (server, _value, port) = server(ServerOptions::new(), None);
            if serve_page {
                server.serve_web(&page.0).unwrap();
            }
            // Like a browser's speculative connection that never sends a request.
            let _idle = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            thread::sleep(Duration::from_millis(50));

            // Peer waits 5s, well below the 10s handshake timeout.
            let mut peer = Peer::connect(port, None);
            peer.wait_for_value(731);
            if serve_page {
                assert!(get(port, "/").ends_with("page"));
            }
            server.stop();
        }
    }

    #[test]
    fn a_request_head_split_across_packets_is_served() {
        if crate::test_support::isolated() {
            return;
        }
        let page = WebDir::new("page");
        let (server, _value, port) = server(ServerOptions::new(), None);
        server.serve_web(&page.0).unwrap();

        let response = http(port, &[b"GET / HTTP/1.1\r\nHo", b"st: localhost\r\n\r\n"]);
        assert!(
            response.starts_with("HTTP/1.1 200") && response.ends_with("page"),
            "{response}"
        );
        server.stop();
    }

    #[test]
    fn web_page_set_before_start_survives_restart() {
        if crate::test_support::isolated() {
            return;
        }
        let page = WebDir::new("page");
        let server = StateServer::new().unwrap();
        server.finalize().unwrap();
        server.serve_web(&page.0).unwrap();

        for _ in 0..2 {
            let port = free_port();
            server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();
            assert!(get(port, "/").ends_with("page"));
            server.stop();
        }
        assert_eq!(server.web_path().as_deref(), Some(page.0.as_path()));
    }
}
