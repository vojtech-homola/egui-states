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
    pub fn new(port: u16) -> Result<Self> {
        Self::with_options(ServerOptions::new(port))
    }

    pub fn with_options(options: ServerOptions) -> Result<Self> {
        let addr = match options.ip_addr {
            Some(addr) => SocketAddrV4::new(addr, options.port),
            None => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, options.port),
        };
        let server = CoreServer::new(addr, options.version, options.token);
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

    pub fn finalize(&self) -> Result<()> {
        self.inner.server.write().finalize();
        Ok(())
    }

    pub fn start(&self) -> Result<()> {
        self.inner
            .server
            .write()
            .start()
            .map_err(ServerError::new)?;
        self.start_signal_workers();
        Ok(())
    }

    pub fn stop(&self) {
        self.inner.server.write().stop();
    }

    pub fn disconnect_client(&self) {
        self.inner.server.write().disconnect_client();
    }

    pub fn is_running(&self) -> bool {
        self.inner.server.read().is_running()
    }

    pub fn is_connected(&self) -> bool {
        self.inner.server.read().is_connected()
    }

    pub fn update(&self, duration: Option<f32>) -> Result<()> {
        self.inner
            .server
            .read()
            .update(duration)
            .map_err(|_| ServerError::new("failed to send update"))
    }

    pub fn set_error_handler(&self, handler: impl Fn(ServerError) + Send + Sync + 'static) {
        *self.inner.error_handler.write() = Arc::new(handler);
    }

    pub fn on_connect(&self, callback: impl Fn(String) + Send + Sync + 'static) -> CallbackHandle {
        self.add_typed_callback(ON_CONNECT_ID, callback)
    }

    pub fn on_disconnect(&self, callback: impl Fn() + Send + Sync + 'static) -> CallbackHandle {
        self.add_raw_callback(ON_DISCONNECT_ID, move |_, _| {
            callback();
            Ok(())
        })
    }

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
        let server = StateServer::new(0).unwrap();
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
        let server = StateServer::new(0).unwrap();

        assert!(server.start().is_err());
        assert!(server.inner.workers.lock().is_empty());
    }

    #[test]
    fn bind_errors_are_reported_and_start_can_be_retried() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut options = ServerOptions::new(port);
        options.ip_addr = Some(Ipv4Addr::LOCALHOST);
        let server = StateServer::with_options(options).unwrap();
        server.finalize().unwrap();

        let error = server.start().unwrap_err();
        assert!(error.message().contains("binding failed"));
        assert!(!server.is_running());
        assert!(server.inner.workers.lock().is_empty());

        drop(listener);
        server.start().unwrap();
        assert!(server.is_running());
        server.stop();
    }

    #[test]
    fn running_server_stops_when_dropped() {
        let server = StateServer::new(0).unwrap();
        server.finalize().unwrap();
        server.start().unwrap();

        drop(server);
    }

    #[cfg(feature = "client")]
    #[test]
    fn rust_client_and_server_exchange_values() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut options = ServerOptions::new(port);
        options.ip_addr = Some(Ipv4Addr::LOCALHOST);
        let server = StateServer::with_options(options).unwrap();
        let server_value = crate::server::Value::new(&server, "root.value", 0_i32, false).unwrap();
        server.finalize().unwrap();
        server.start().unwrap();

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

    #[test]
    fn signal_workers_dispatch_and_shutdown() {
        let mut options = ServerOptions::new(0);
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
        let mut options = ServerOptions::new(0);
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
        let mut options = ServerOptions::new(0);
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
        let server = StateServer::new(0).unwrap();
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
        let server = StateServer::new(0).unwrap();
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
        let (release_sender, release_receiver) = mpsc::channel::<()>();
        let (entered_sender, entered_receiver) = mpsc::channel::<()>();

        let mut options = ServerOptions::new(0);
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
        let mut options = ServerOptions::new(0);
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
