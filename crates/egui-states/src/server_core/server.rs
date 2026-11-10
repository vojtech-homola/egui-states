use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::runtime::Builder;

use crate::data_transport::DataType;
use crate::event::Event;
use crate::hashing::{MAX_RESERVED_ID, NoHashMap, generate_value_id};
use crate::serialization::{ServerHeader, check_value_size, serialize};
use crate::server_core::core;
use crate::server_core::data_core::{Data, DataMulti};
use crate::server_core::data_take_core::{DataMultiTake, DataTake};
use crate::server_core::heartbeat::Heartbeat;
use crate::server_core::image_core::Image;
use crate::server_core::image_multi_core::ImageMulti;
use crate::server_core::map_core::ValueMap;
use crate::server_core::origin::OriginPolicy;
use crate::server_core::sender::{MessageReceiver, MessageSender};
use crate::server_core::signals::SignalsManager;
use crate::server_core::values_core::{SignalCore, ValueCore, ValueStaticCore, ValueTakeCore};
use crate::server_core::vec_core::ValueList;
use crate::server_core::web::{WebSite, WebSlot};

pub(crate) trait SyncTrait: Sync + Send {
    fn sync(&self) -> Result<(), ()>;
}

pub(crate) trait Acknowledge: Sync + Send {
    fn acknowledge(&self);

    fn reset(&self) {
        self.acknowledge();
    }
}

/// A registered state of any kind.
#[derive(Clone)]
pub(crate) enum StateRef {
    Value(Arc<ValueCore>),
    ValueTake(Arc<ValueTakeCore>),
    Static(Arc<ValueStaticCore>),
    Signal(Arc<SignalCore>),
    Vec(Arc<ValueList>),
    Map(Arc<ValueMap>),
    Image(Arc<Image>),
    ImageMulti(Arc<ImageMulti>),
    Data(Arc<Data>),
    DataMulti(Arc<DataMulti>),
    DataTake(Arc<DataTake>),
    DataMultiTake(Arc<DataMultiTake>),
}

impl StateRef {
    /// Where the kind is synchronized with a newly connected client; kinds
    /// go in this order, images before data. Signals are not synchronized.
    fn sync_rank(&self) -> Option<u8> {
        Some(match self {
            Self::ValueTake(_) => 0,
            Self::Value(_) => 1,
            Self::Static(_) => 2,
            Self::Image(_) => 3,
            Self::ImageMulti(_) => 4,
            Self::Map(_) => 5,
            Self::Vec(_) => 6,
            Self::Data(_) => 7,
            Self::DataMulti(_) => 8,
            Self::DataTake(_) => 9,
            Self::DataMultiTake(_) => 10,
            Self::Signal(_) => return None,
        })
    }

    fn sync(&self) -> Option<Arc<dyn SyncTrait>> {
        Some(match self {
            Self::Value(state) => state.clone(),
            Self::ValueTake(state) => state.clone(),
            Self::Static(state) => state.clone(),
            Self::Vec(state) => state.clone(),
            Self::Map(state) => state.clone(),
            Self::Image(state) => state.clone(),
            Self::ImageMulti(state) => state.clone(),
            Self::Data(state) => state.clone(),
            Self::DataMulti(state) => state.clone(),
            Self::DataTake(state) => state.clone(),
            Self::DataMultiTake(state) => state.clone(),
            Self::Signal(_) => return None,
        })
    }

    /// The kinds the client acknowledges.
    fn ack(&self) -> Option<Arc<dyn Acknowledge>> {
        Some(match self {
            Self::Value(state) => state.clone(),
            Self::ValueTake(state) => state.clone(),
            Self::Image(state) => state.clone(),
            Self::ImageMulti(state) => state.clone(),
            Self::Data(state) => state.clone(),
            Self::DataMulti(state) => state.clone(),
            Self::DataTake(state) => state.clone(),
            Self::DataMultiTake(state) => state.clone(),
            Self::Static(_) | Self::Vec(_) | Self::Map(_) | Self::Signal(_) => return None,
        })
    }
}

/// Every registered state, by id. An id belongs to one state of one kind.
#[derive(Clone, Default)]
pub(crate) struct StatesList {
    pub(crate) states: NoHashMap<u64, StateRef>,
}

impl StatesList {
    fn contains_id(&self, id: u64) -> bool {
        id <= MAX_RESERVED_ID || self.states.contains_key(&id)
    }

    fn get_server_list(&self) -> ServerStatesList {
        let mut server_list = ServerStatesList::default();
        let mut sync = Vec::new();
        for (id, state) in self.states.iter() {
            match state {
                StateRef::Value(value) => {
                    server_list.values.insert(*id, value.clone());
                }
                StateRef::Signal(signal) => {
                    server_list.signals.insert(*id, signal.clone());
                }
                _ => {}
            }
            if let Some(ack) = state.ack() {
                server_list.ack.insert(*id, ack);
            }
            if let (Some(rank), Some(state)) = (state.sync_rank(), state.sync()) {
                sync.push((rank, state));
            }
        }
        // Stable, so the states of one kind keep the map's order.
        sync.sort_by_key(|(rank, _)| *rank);
        server_list.sync = sync.into_iter().map(|(_, state)| state).collect();
        server_list
    }
}

/// Typed lookups of registered states; a state of another kind is reported
/// like a missing one.
#[cfg(feature = "server")]
macro_rules! getters {
    ($($name:ident: $variant:ident($ty:ty)),* $(,)?) => {
        $(
            pub(crate) fn $name(&self, id: u64) -> Option<Arc<$ty>> {
                match self.states.states.get(&id) {
                    Some(StateRef::$variant(state)) => Some(state.clone()),
                    _ => None,
                }
            }
        )*
    };
}

/// The data type for a protocol type id.
fn data_type(type_id: u8) -> Result<DataType, String> {
    DataType::from_id(type_id).map_err(|_| "Invalid data type id".to_string())
}

#[derive(Clone, Default)]
pub(crate) struct ServerStatesList {
    pub(crate) values: NoHashMap<u64, Arc<ValueCore>>,
    pub(crate) signals: NoHashMap<u64, Arc<SignalCore>>,
    pub(crate) ack: NoHashMap<u64, Arc<dyn Acknowledge>>,
    pub(crate) sync: Vec<Arc<dyn SyncTrait>>,
}

/// Why [`Server::start`] failed.
#[derive(Debug)]
pub(crate) enum StartError {
    /// The server is in the wrong lifecycle phase: not finalized, in an
    /// undefined state, or already running with other settings.
    InvalidState(String),
    /// The socket could not be bound or configured, or the runtime or thread
    /// could not be started.
    Io(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::InvalidState(message) | StartError::Io(message) => f.write_str(message),
        }
    }
}

/// Settings of a running server.
struct Listening {
    /// The address `start` was called with; its port may be 0.
    requested: SocketAddrV4,
    /// The address the listener is bound to.
    bound: SocketAddrV4,
    token: Option<String>,
}

impl Listening {
    /// Checks a repeated `start` against the active settings.
    ///
    /// The address matches the requested one or the bound one, so a server
    /// started on port 0 also accepts the port it was given. The message never
    /// contains a token.
    fn check(&self, addr: SocketAddrV4, token: &Option<String>) -> Result<(), StartError> {
        if addr != self.requested && addr != self.bound {
            return Err(StartError::InvalidState(format!(
                "the server is already running on {}; stop it before starting it on {addr}",
                self.bound
            )));
        }
        if *token != self.token {
            return Err(StartError::InvalidState(
                "the server is already running with a different token; stop it before \
                 changing the token"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

enum RunnerState {
    Running(thread::JoinHandle<MessageReceiver>, Listening),
    Stopped(MessageReceiver),
    Undefined,
}

impl RunnerState {
    fn take(&mut self) -> Self {
        std::mem::replace(self, RunnerState::Undefined)
    }
}

pub(crate) struct Server {
    connected: Arc<AtomicBool>,
    stop_event: Event,
    sender: MessageSender,
    states: StatesList,
    states_server: Option<ServerStatesList>,
    signals: SignalsManager,
    version: Option<u64>,
    web_path: Option<PathBuf>,
    web: WebSlot,
    origins: Arc<OriginPolicy>,
    heartbeat: Option<Heartbeat>,

    /// Held by `start` and `stop` for their whole run, including the join of
    /// the server thread, so they never overlap. Nothing that a connected
    /// client needs, like `update` or `is_connected`, takes it.
    runner_state: Mutex<RunnerState>,
}

/// Binds `addr` and builds the runtime whose thread will serve it.
///
/// Returns the runtime, the listener registered with it and the bound address.
fn listen(
    addr: SocketAddrV4,
) -> Result<(tokio::runtime::Runtime, TcpListener, SocketAddrV4), StartError> {
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|error| StartError::Io(format!("binding failed: {error:?}")))?;
    let bound = match listener.local_addr() {
        Ok(std::net::SocketAddr::V4(bound)) => bound,
        // Binding an IPv4 address always reports an IPv4 one.
        Ok(std::net::SocketAddr::V6(_)) => addr,
        Err(error) => {
            return Err(StartError::Io(format!(
                "failed to read the listener address: {error:?}"
            )));
        }
    };
    listener
        .set_nonblocking(true)
        .map_err(|error| StartError::Io(format!("failed to configure listener: {error:?}")))?;

    let runtime = Builder::new_multi_thread()
        .thread_name("ServerRuntime")
        .enable_io()
        .enable_time()
        .worker_threads(2)
        .thread_keep_alive(Duration::from_hours(1))
        .build()
        .map_err(|_| StartError::Io("Failed to create server runtime".to_string()))?;
    let listener = {
        let _guard = runtime.enter();
        TcpListener::from_std(listener)
    }
    .map_err(|error| StartError::Io(format!("failed to initialize listener: {error:?}")))?;
    Ok((runtime, listener, bound))
}

impl Server {
    pub(crate) fn new(version: Option<u64>) -> Self {
        let connected = Arc::new(AtomicBool::new(false));
        let (sender, rx) = MessageSender::new();
        let signals = SignalsManager::new();

        Self {
            connected,
            stop_event: Event::new(),
            sender,
            states: StatesList::default(),
            states_server: None,
            signals,
            version,
            web_path: None,
            web: WebSlot::default(),
            origins: Arc::default(),
            heartbeat: Some(Heartbeat::default()),
            runner_state: Mutex::new(RunnerState::Stopped(rx)),
        }
    }

    /// Sets the heartbeat used from the next [`Self::start`]; `None` disables it.
    pub(crate) fn set_heartbeat(&mut self, heartbeat: Option<Heartbeat>) {
        self.heartbeat = heartbeat;
    }

    /// Sets the web page directory that `start_web` loads and serves.
    ///
    /// A relative path is resolved against the current directory now, so a
    /// later change of the current directory does not move the page. The
    /// directory itself is not read here.
    pub(crate) fn set_web_path(&mut self, path: Option<&Path>) -> Result<(), String> {
        self.web_path = path
            .map(|path| {
                std::path::absolute(path)
                    .map_err(|e| format!("invalid web page path {}: {e}", path.display()))
            })
            .transpose()?;
        Ok(())
    }

    /// Sets which browser pages may connect, besides pages served by the
    /// server itself on an IP address or local network name.
    ///
    /// `origins` are `scheme://host[:port]` patterns, where `*` matches any
    /// scheme, part of the host, or port. Applies from the next start.
    pub(crate) fn set_allowed_origins(&mut self, origins: &[String]) -> Result<(), String> {
        self.origins = Arc::new(OriginPolicy::new(origins)?);
        Ok(())
    }

    pub(crate) fn finalize(&mut self) -> Option<StatesList> {
        match self.states_server {
            Some(_) => None,
            None => {
                let states_server = self.states.get_server_list();
                self.states_server = Some(states_server);
                Some(self.states.clone())
            }
        }
    }

    pub(crate) fn get_signals_manager(&self) -> SignalsManager {
        self.signals.clone()
    }

    /// Starts the server on its dedicated thread.
    ///
    /// Starting a running server with the same address and token succeeds
    /// without effect; other settings return [`StartError::InvalidState`].
    ///
    /// This method must not be called from inside a Tokio runtime. The server
    /// creates and owns a Tokio runtime, which must be allowed to shut down on
    /// a non-async thread.
    pub(crate) fn start(
        &self,
        addr: SocketAddrV4,
        token: Option<String>,
    ) -> Result<(), StartError> {
        let mut state = self.runner_state.lock();
        let runner_state = match state.take() {
            RunnerState::Running(handle, _) if handle.is_finished() => match handle.join() {
                Ok(rx) => RunnerState::Stopped(rx),
                Err(_) => RunnerState::Undefined,
            },
            state => state,
        };

        match (runner_state, &self.states_server) {
            (RunnerState::Running(handle, listening), _) => {
                let checked = listening.check(addr, &token);
                *state = RunnerState::Running(handle, listening);
                checked
            }
            (RunnerState::Stopped(rx), Some(states_server)) => {
                let (runtime, listener, bound) = match listen(addr) {
                    Ok(listening) => listening,
                    Err(error) => {
                        *state = RunnerState::Stopped(rx);
                        return Err(error);
                    }
                };

                let sender = self.sender.clone();
                let connected = self.connected.clone();
                let stop_event = self.stop_event.clone();
                let values = states_server.clone();
                let signals = self.signals.clone();
                let web = self.web.clone();
                let heartbeat = self.heartbeat;

                let listening = Listening {
                    requested: addr,
                    bound,
                    token: token.clone(),
                };
                let handshake = core::Handshake {
                    version: self.version,
                    token,
                    origins: self.origins.clone(),
                };

                let server_thread = thread::Builder::new().name("StatesServer".to_string());
                stop_event.clear();
                let thread_handle_res = server_thread.spawn(move || {
                    runtime.block_on(async move {
                        let session = core::Session {
                            sender,
                            connected,
                            values,
                            signals,
                            heartbeat,
                        };
                        core::run(listener, rx, stop_event, handshake, web, session).await
                    })
                });

                match thread_handle_res {
                    Err(_) => {
                        *state = RunnerState::Undefined;
                        Err(StartError::Io(
                            "Failed to start server thread, server is in undefined state"
                                .to_string(),
                        ))
                    }
                    Ok(thread_handle) => {
                        *state = RunnerState::Running(thread_handle, listening);
                        Ok(())
                    }
                }
            }
            (RunnerState::Undefined, _) => Err(StartError::InvalidState(
                "Server is in undefined state".to_string(),
            )),
            (stopped, None) => {
                *state = stopped;
                Err(StartError::InvalidState(
                    "Server has not been finalized".to_string(),
                ))
            }
        }
    }

    /// Stops the server and waits for its thread to finish.
    pub(crate) fn stop(&self) {
        let mut state = self.runner_state.lock();
        match state.take() {
            RunnerState::Stopped(rx) => {
                *state = RunnerState::Stopped(rx);
            }
            RunnerState::Running(handle, _) => {
                self.connected.store(false, Ordering::Release);
                self.stop_event.set();
                self.sender.close();

                *state = match handle.join() {
                    Ok(rx) => RunnerState::Stopped(rx),
                    Err(_) => RunnerState::Undefined,
                };
            }
            RunnerState::Undefined => {}
        }
    }

    pub(crate) fn disconnect_client(&self) {
        if self.connected.load(Ordering::Acquire) {
            self.connected.store(false, Ordering::Release);
            self.sender.close();
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.address().is_some()
    }

    /// Returns the address the server listens on while it is running.
    ///
    /// With port 0 this is the port the system assigned. While `stop` is
    /// joining the server thread this waits for it, unless `stop` has already
    /// signalled the thread.
    pub(crate) fn address(&self) -> Option<SocketAddrV4> {
        if self.stop_event.is_set() {
            return None;
        }
        match &*self.runner_state.lock() {
            RunnerState::Running(handle, listening)
                if !handle.is_finished() && !self.stop_event.is_set() =>
            {
                Some(listening.bound)
            }
            _ => None,
        }
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// Replaces the served web page, or stops serving it with `None`.
    ///
    /// The setting applies to connections accepted afterwards and is kept
    /// while the server is stopped.
    pub(crate) fn set_web(&self, site: Option<WebSite>) {
        *self.web.write() = site.map(Arc::new);
    }

    /// Returns whether a web page is being served.
    pub(crate) fn is_web_running(&self) -> bool {
        self.web.read().is_some()
    }

    /// Returns the configured absolute web page directory, served or not.
    pub(crate) fn web_path(&self) -> Option<PathBuf> {
        self.web_path.clone()
    }

    pub(crate) fn update(&self, duration: Option<f32>) -> Result<(), ()> {
        if self.connected.load(Ordering::Acquire) {
            let duration = duration.unwrap_or(0.0);
            let header = ServerHeader::Update(duration);
            let data = serialize(&header)?;
            self.sender.send(data);
        }
        Ok(())
    }

    /// Registers the state `make` builds for `name`'s id; `kind` names the
    /// state in the duplicate error.
    fn register(
        &mut self,
        name: &str,
        kind: &str,
        make: impl FnOnce(u64) -> Result<StateRef, String>,
    ) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("{kind} with id {id} already exists"));
        }
        let state = make(id)?;
        self.states.states.insert(id, state);
        Ok(id)
    }

    /// What a new state needs to send and to raise signals.
    fn channels(&self) -> (MessageSender, Arc<AtomicBool>, SignalsManager) {
        (
            self.sender.clone(),
            self.connected.clone(),
            self.signals.clone(),
        )
    }

    pub(crate) fn add_value(
        &mut self,
        name: &str,
        type_id: u32,
        value: Bytes,
        queue: bool,
    ) -> Result<u64, String> {
        let (sender, connected, signals) = self.channels();
        let id = self.register(name, "Value", |id| {
            check_value_size(name, value.len())?;
            Ok(StateRef::Value(ValueCore::new(
                name.to_string(),
                id,
                type_id,
                value,
                sender,
                connected,
                signals,
            )))
        })?;
        if queue {
            self.signals.set_to_queue(id);
        }
        Ok(id)
    }

    pub(crate) fn add_value_take(&mut self, name: &str, type_id: u32) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "ValueTake", |id| {
            Ok(StateRef::ValueTake(ValueTakeCore::new(
                name.to_string(),
                id,
                type_id,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_static(
        &mut self,
        name: &str,
        type_id: u32,
        value: Bytes,
    ) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "Static value", |id| {
            check_value_size(name, value.len())?;
            Ok(StateRef::Static(ValueStaticCore::new(
                name.to_string(),
                id,
                type_id,
                value,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_signal(
        &mut self,
        name: &str,
        type_id: u32,
        queue: bool,
    ) -> Result<u64, String> {
        let (_, _, signals) = self.channels();
        let id = self.register(name, "Signal", |id| {
            Ok(StateRef::Signal(SignalCore::new(
                name.to_string(),
                id,
                type_id,
                signals,
            )))
        })?;
        if queue {
            self.signals.set_to_queue(id);
        }
        Ok(id)
    }

    pub(crate) fn add_vec(&mut self, name: &str, type_id: u32) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "Vec", |id| {
            Ok(StateRef::Vec(ValueList::new(
                name.to_string(),
                id,
                type_id,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_map(&mut self, name: &str, type_id: u32) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "Map", |id| {
            Ok(StateRef::Map(ValueMap::new(
                name.to_string(),
                id,
                type_id,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_image(&mut self, name: &str) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "Image", |id| {
            Ok(StateRef::Image(Image::new(
                name.to_string(),
                id,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_image_multi(&mut self, name: &str) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "ImageMulti", |id| {
            Ok(StateRef::ImageMulti(ImageMulti::new(
                name.to_string(),
                id,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_data(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "Data", |id| {
            Ok(StateRef::Data(Data::new(
                name.to_string(),
                id,
                data_type(type_id)?,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_data_multi(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "DataMulti", |id| {
            Ok(StateRef::DataMulti(DataMulti::new(
                name.to_string(),
                id,
                data_type(type_id)?,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_data_take(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "DataTake", |id| {
            Ok(StateRef::DataTake(DataTake::new(
                name.to_string(),
                id,
                data_type(type_id)?,
                sender,
                connected,
            )))
        })
    }

    pub(crate) fn add_data_multi_take(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        let (sender, connected, _) = self.channels();
        self.register(name, "DataMultiTake", |id| {
            Ok(StateRef::DataMultiTake(DataMultiTake::new(
                name.to_string(),
                id,
                data_type(type_id)?,
                sender,
                connected,
            )))
        })
    }

    #[cfg(feature = "server")]
    getters! {
        get_value: Value(ValueCore),
        get_value_take: ValueTake(ValueTakeCore),
        get_static: Static(ValueStaticCore),
        get_signal: Signal(SignalCore),
        get_vec: Vec(ValueList),
        get_map: Map(ValueMap),
        get_image: Image(Image),
        get_image_multi: ImageMulti(ImageMulti),
        get_data: Data(Data),
        get_data_take: DataTake(DataTake),
        get_data_multi: DataMulti(DataMulti),
        get_data_multi_take: DataMultiTake(DataMultiTake),
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;

    use super::*;

    /// `stop` holds the runner lock while it joins the server thread, and the
    /// front-ends call it under a shared lock. The calls a connected client
    /// needs must not wait for it.
    #[test]
    fn client_calls_do_not_wait_for_a_stop_in_progress() {
        let server = Arc::new(Server::new(None));
        let stopping = server.runner_state.lock();

        let (done, finished) = mpsc::channel();
        let caller = server.clone();
        thread::spawn(move || {
            caller.update(None).unwrap();
            caller.is_connected();
            caller.disconnect_client();
            let _ = done.send(());
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(2)).is_ok(),
            "a client call waited for the stop"
        );
        drop(stopping);
    }
}
