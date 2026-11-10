use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use bytes::Bytes;
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

#[derive(Clone, Default)]
pub(crate) struct StatesList {
    pub(crate) values: NoHashMap<u64, Arc<ValueCore>>,
    pub(crate) values_take: NoHashMap<u64, Arc<ValueTakeCore>>,
    pub(crate) static_values: NoHashMap<u64, Arc<ValueStaticCore>>,
    pub(crate) signals: NoHashMap<u64, Arc<SignalCore>>,
    pub(crate) images: NoHashMap<u64, Arc<Image>>,
    pub(crate) image_multi: NoHashMap<u64, Arc<ImageMulti>>,
    pub(crate) maps: NoHashMap<u64, Arc<ValueMap>>,
    pub(crate) lists: NoHashMap<u64, Arc<ValueList>>,
    pub(crate) data: NoHashMap<u64, Arc<Data>>,
    pub(crate) data_take: NoHashMap<u64, Arc<DataTake>>,
    pub(crate) data_multi: NoHashMap<u64, Arc<DataMulti>>,
    pub(crate) data_multi_take: NoHashMap<u64, Arc<DataMultiTake>>,
}

impl StatesList {
    fn contains_id(&self, id: u64) -> bool {
        id <= MAX_RESERVED_ID
            || self.values.contains_key(&id)
            || self.values_take.contains_key(&id)
            || self.static_values.contains_key(&id)
            || self.signals.contains_key(&id)
            || self.images.contains_key(&id)
            || self.image_multi.contains_key(&id)
            || self.maps.contains_key(&id)
            || self.lists.contains_key(&id)
            || self.data.contains_key(&id)
            || self.data_take.contains_key(&id)
            || self.data_multi.contains_key(&id)
            || self.data_multi_take.contains_key(&id)
    }

    fn get_server_list(&self) -> ServerStatesList {
        let mut server_list = ServerStatesList::default();

        server_list.values.extend(self.values.clone());
        server_list.signals.extend(self.signals.clone());

        for (id, value_take) in self.values_take.iter() {
            server_list.sync.push(value_take.clone());
            server_list.ack.insert(*id, value_take.clone());
        }

        for (id, value) in self.values.iter() {
            server_list.sync.push(value.clone());
            server_list.ack.insert(*id, value.clone());
        }

        for value in self.static_values.values() {
            server_list.sync.push(value.clone());
        }

        for (id, image) in self.images.iter() {
            server_list.sync.push(image.clone());
            server_list.ack.insert(*id, image.clone());
        }

        for (id, images) in self.image_multi.iter() {
            server_list.sync.push(images.clone());
            server_list.ack.insert(*id, images.clone());
        }

        for map in self.maps.values() {
            server_list.sync.push(map.clone());
        }

        for list in self.lists.values() {
            server_list.sync.push(list.clone());
        }

        for (id, data) in self.data.iter() {
            server_list.sync.push(data.clone());
            server_list.ack.insert(*id, data.clone());
        }

        for (id, data_multi) in self.data_multi.iter() {
            server_list.sync.push(data_multi.clone());
            server_list.ack.insert(*id, data_multi.clone());
        }

        for (id, data_take) in self.data_take.iter() {
            server_list.sync.push(data_take.clone());
            server_list.ack.insert(*id, data_take.clone());
        }

        for (id, data_multi_take) in self.data_multi_take.iter() {
            server_list.sync.push(data_multi_take.clone());
            server_list.ack.insert(*id, data_multi_take.clone());
        }

        server_list
    }
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

    runner_state: RunnerState,
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
            runner_state: RunnerState::Stopped(rx),
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
        &mut self,
        addr: SocketAddrV4,
        token: Option<String>,
    ) -> Result<(), StartError> {
        let runner_state = match self.runner_state.take() {
            RunnerState::Running(handle, _) if handle.is_finished() => match handle.join() {
                Ok(rx) => RunnerState::Stopped(rx),
                Err(_) => RunnerState::Undefined,
            },
            state => state,
        };

        match (runner_state, &self.states_server) {
            (RunnerState::Running(handle, listening), _) => {
                let checked = listening.check(addr, &token);
                self.runner_state = RunnerState::Running(handle, listening);
                checked
            }
            (RunnerState::Stopped(rx), Some(states_server)) => {
                let (runtime, listener, bound) = match listen(addr) {
                    Ok(listening) => listening,
                    Err(error) => {
                        self.runner_state = RunnerState::Stopped(rx);
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
                        self.runner_state = RunnerState::Undefined;
                        Err(StartError::Io(
                            "Failed to start server thread, server is in undefined state"
                                .to_string(),
                        ))
                    }
                    Ok(thread_handle) => {
                        self.runner_state = RunnerState::Running(thread_handle, listening);
                        Ok(())
                    }
                }
            }
            (RunnerState::Undefined, _) => Err(StartError::InvalidState(
                "Server is in undefined state".to_string(),
            )),
            (state, None) => {
                self.runner_state = state;
                Err(StartError::InvalidState(
                    "Server has not been finalized".to_string(),
                ))
            }
        }
    }

    pub(crate) fn stop(&mut self) {
        match self.runner_state.take() {
            RunnerState::Stopped(rx) => {
                self.runner_state = RunnerState::Stopped(rx);
            }
            RunnerState::Running(handle, _) => {
                self.connected.store(false, Ordering::Release);
                self.stop_event.set();
                self.sender.close();

                match handle.join() {
                    Ok(rx) => {
                        self.runner_state = RunnerState::Stopped(rx);
                    }
                    Err(_) => {
                        self.runner_state = RunnerState::Undefined;
                    }
                }
            }
            RunnerState::Undefined => {}
        }
    }

    pub(crate) fn disconnect_client(&mut self) {
        if self.connected.load(Ordering::Acquire) {
            self.connected.store(false, Ordering::Release);
            self.sender.close();
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.listening().is_some()
    }

    /// Returns the address the server listens on while it is running.
    ///
    /// With port 0 this is the port the system assigned.
    pub(crate) fn address(&self) -> Option<SocketAddrV4> {
        self.listening().map(|listening| listening.bound)
    }

    fn listening(&self) -> Option<&Listening> {
        match &self.runner_state {
            RunnerState::Running(handle, listening)
                if !handle.is_finished() && !self.stop_event.is_set() =>
            {
                Some(listening)
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

    pub(crate) fn add_value(
        &mut self,
        name: &str,
        type_id: u32,
        value: Bytes,
        queue: bool,
    ) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("Value with id {} already exists", id));
        }
        check_value_size(name, value.len())?;

        let val = ValueCore::new(
            name.to_string(),
            id,
            type_id,
            value,
            self.sender.clone(),
            self.connected.clone(),
            self.signals.clone(),
        );

        self.states.values.insert(id, val);

        if queue {
            self.signals.set_to_queue(id);
        }

        Ok(id)
    }

    pub(crate) fn add_value_take(&mut self, name: &str, type_id: u32) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("ValueTake with id {} already exists", id));
        }

        let val = ValueTakeCore::new(
            name.to_string(),
            id,
            type_id,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.values_take.insert(id, val);

        Ok(id)
    }

    pub(crate) fn add_static(
        &mut self,
        name: &str,
        type_id: u32,
        value: Bytes,
    ) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("Static value with id {} already exists", id));
        }
        check_value_size(name, value.len())?;

        let val = ValueStaticCore::new(
            name.to_string(),
            id,
            type_id,
            value,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.static_values.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_signal(
        &mut self,
        name: &str,
        type_id: u32,
        queue: bool,
    ) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("Signal with id {} already exists", id));
        }

        let val = SignalCore::new(name.to_string(), id, type_id, self.signals.clone());

        self.states.signals.insert(id, val);

        if queue {
            self.signals.set_to_queue(id);
        }

        Ok(id)
    }

    pub(crate) fn add_vec(&mut self, name: &str, type_id: u32) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("Vec with id {} already exists", id));
        }

        let val = ValueList::new(
            name.to_string(),
            id,
            type_id,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.lists.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_map(&mut self, name: &str, type_id: u32) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("Map with id {} already exists", id));
        }

        let val = ValueMap::new(
            name.to_string(),
            id,
            type_id,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.maps.insert(id, val);

        Ok(id)
    }

    pub(crate) fn add_image(&mut self, name: &str) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("Image with id {} already exists", id));
        }

        let val = Image::new(
            name.to_string(),
            id,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.images.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_image_multi(&mut self, name: &str) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("ImageMulti with id {} already exists", id));
        }

        let value = ImageMulti::new(
            name.to_string(),
            id,
            self.sender.clone(),
            self.connected.clone(),
        );
        self.states.image_multi.insert(id, value);
        Ok(id)
    }

    pub(crate) fn add_data(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("Data with id {} already exists", id));
        }

        let data_type =
            DataType::from_id(type_id).map_err(|_| "Invalid data type id".to_string())?;
        let val = Data::new(
            name.to_string(),
            id,
            data_type,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.data.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_data_multi(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("DataMulti with id {} already exists", id));
        }

        let data_type =
            DataType::from_id(type_id).map_err(|_| "Invalid data type id".to_string())?;
        let val = DataMulti::new(
            name.to_string(),
            id,
            data_type,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.data_multi.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_data_take(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("DataTake with id {} already exists", id));
        }

        let data_type =
            DataType::from_id(type_id).map_err(|_| "Invalid data type id".to_string())?;
        let val = DataTake::new(
            name.to_string(),
            id,
            data_type,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.data_take.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_data_multi_take(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(name);
        if self.states.contains_id(id) {
            return Err(format!("DataMultiTake with id {} already exists", id));
        }

        let data_type =
            DataType::from_id(type_id).map_err(|_| "Invalid data type id".to_string())?;
        let val = DataMultiTake::new(
            name.to_string(),
            id,
            data_type,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.data_multi_take.insert(id, val);
        Ok(id)
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_value(&self, id: u64) -> Option<Arc<ValueCore>> {
        self.states.values.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_value_take(&self, id: u64) -> Option<Arc<ValueTakeCore>> {
        self.states.values_take.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_static(&self, id: u64) -> Option<Arc<ValueStaticCore>> {
        self.states.static_values.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_signal(&self, id: u64) -> Option<Arc<SignalCore>> {
        self.states.signals.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_vec(&self, id: u64) -> Option<Arc<ValueList>> {
        self.states.lists.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_map(&self, id: u64) -> Option<Arc<ValueMap>> {
        self.states.maps.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_image(&self, id: u64) -> Option<Arc<Image>> {
        self.states.images.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_image_multi(&self, id: u64) -> Option<Arc<ImageMulti>> {
        self.states.image_multi.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_data(&self, id: u64) -> Option<Arc<Data>> {
        self.states.data.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_data_take(&self, id: u64) -> Option<Arc<DataTake>> {
        self.states.data_take.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_data_multi(&self, id: u64) -> Option<Arc<DataMulti>> {
        self.states.data_multi.get(&id).cloned()
    }

    #[cfg(feature = "server")]
    pub(crate) fn get_data_multi_take(&self, id: u64) -> Option<Arc<DataMultiTake>> {
        self.states.data_multi_take.get(&id).cloned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
