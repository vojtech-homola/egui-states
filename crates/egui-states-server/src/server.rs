use std::net::SocketAddrV4;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

use bytes::Bytes;
use tokio::runtime::Builder;

use egui_states_core::event_async::Event;
use egui_states_core::generate_value_id;
use egui_states_core::graphs::GraphType;
use egui_states_core::nohash::NoHashMap;
use egui_states_core::serialization::ServerHeader;

use crate::graphs::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::map::ValueMap;
use crate::sender::{MessageReceiver, MessageSender};
use crate::server_core;
use crate::signals::SignalsManager;
use crate::values::{Signal, Value, ValueStatic};

pub(crate) trait SyncTrait: Sync + Send {
    fn sync(&self);
}

pub(crate) trait EnableTrait: Sync + Send {
    fn enable(&self, enable: bool);
}

pub(crate) trait Acknowledge: Sync + Send {
    fn acknowledge(&self);
}

#[derive(Clone, Default)]
pub(crate) struct StatesList {
    pub(crate) values: NoHashMap<u64, Arc<Value>>,
    pub(crate) static_values: NoHashMap<u64, Arc<ValueStatic>>,
    pub(crate) signals: NoHashMap<u64, Arc<Signal>>,
    pub(crate) images: NoHashMap<u64, Arc<ValueImage>>,
    pub(crate) maps: NoHashMap<u64, Arc<ValueMap>>,
    pub(crate) lists: NoHashMap<u64, Arc<ValueList>>,
    pub(crate) graphs: NoHashMap<u64, Arc<ValueGraphs>>,
    pub(crate) types: NoHashMap<u64, u64>,
}

impl StatesList {
    fn get_server_list(&self) -> ServerStatesList {
        let mut server_list = ServerStatesList::default();

        server_list.types = self.types.clone();
        server_list.values.extend(self.values.clone());
        server_list.signals.extend(self.signals.clone());

        for (id, value) in self.values.iter() {
            server_list.sync.push(value.clone());
            server_list.ack.insert(*id, value.clone());
            server_list.enable.insert(*id, value.clone());
        }

        for (id, value) in self.static_values.iter() {
            server_list.enable.insert(*id, value.clone());
            server_list.sync.push(value.clone());
        }

        for (id, signal) in self.signals.iter() {
            server_list.enable.insert(*id, signal.clone());
        }

        for (id, image) in self.images.iter() {
            server_list.sync.push(image.clone());
            server_list.ack.insert(*id, image.clone());
            server_list.enable.insert(*id, image.clone());
        }

        for (id, map) in self.maps.iter() {
            server_list.enable.insert(*id, map.clone());
            server_list.sync.push(map.clone());
        }

        for (id, list) in self.lists.iter() {
            server_list.enable.insert(*id, list.clone());
            server_list.sync.push(list.clone());
        }

        for (id, graphs) in self.graphs.iter() {
            server_list.enable.insert(*id, graphs.clone());
            server_list.sync.push(graphs.clone());
        }

        server_list
    }
}

#[derive(Clone, Default)]
pub(crate) struct ServerStatesList {
    pub(crate) values: NoHashMap<u64, Arc<Value>>,
    pub(crate) signals: NoHashMap<u64, Arc<Signal>>,
    pub(crate) ack: NoHashMap<u64, Arc<dyn Acknowledge>>,
    pub(crate) enable: NoHashMap<u64, Arc<dyn EnableTrait>>,
    pub(crate) sync: Vec<Arc<dyn SyncTrait>>,
    pub(crate) types: NoHashMap<u64, u64>,
}

enum RunnerState {
    Running(thread::JoinHandle<MessageReceiver>),
    Stopped(MessageReceiver),
    Undefined,
}

impl RunnerState {
    fn take(&mut self) -> Self {
        match std::mem::replace(self, RunnerState::Undefined) {
            RunnerState::Running(handle) => RunnerState::Running(handle),
            RunnerState::Stopped(rx) => RunnerState::Stopped(rx),
            RunnerState::Undefined => RunnerState::Undefined,
        }
    }
}

pub(crate) struct Server {
    connected: Arc<AtomicBool>,
    stop_event: Event,
    sender: MessageSender,
    addr: SocketAddrV4,
    states: StatesList,
    states_server: Option<ServerStatesList>,
    signals: SignalsManager,
    handshake: Option<Vec<u64>>,

    runner_state: RunnerState,
    runner_threads: usize,
}

impl Server {
    pub(crate) fn new(
        addr: SocketAddrV4,
        handshake: Option<Vec<u64>>,
        runner_threads: usize,
    ) -> Self {
        let connected = Arc::new(AtomicBool::new(false));
        let (sender, rx) = MessageSender::new();
        let signals = SignalsManager::new();

        let obj = Self {
            connected,
            stop_event: Event::new(),
            sender,
            addr,
            states: StatesList::default(),
            states_server: None,
            signals,
            handshake,
            runner_state: RunnerState::Stopped(rx),
            runner_threads,
        };

        obj
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

    pub(crate) fn start(&mut self) -> Result<(), &'static str> {
        match (self.runner_state.take(), &self.states_server) {
            (RunnerState::Running(rx), _) => {
                self.runner_state = RunnerState::Running(rx);
                Ok(())
            }
            (RunnerState::Stopped(rx), Some(states_server)) => {
                let runtime = Builder::new_current_thread()
                    .thread_name("ServerRuntime")
                    .enable_io()
                    .worker_threads(self.runner_threads)
                    .build()
                    .unwrap();

                let sender = self.sender.clone();
                let connected = self.connected.clone();
                let stop_event = self.stop_event.clone();
                let values = states_server.clone();
                let signals = self.signals.clone();

                let handshake = self.handshake.clone();
                let addr = self.addr;

                let server_thread = thread::Builder::new().name("StatesServer".to_string());
                stop_event.clear();
                let thread_handle_res = server_thread.spawn(move || {
                    runtime.block_on(async move {
                        server_core::run(
                            sender, rx, connected, stop_event, values, signals, addr, handshake,
                        )
                        .await
                    })
                });

                match thread_handle_res {
                    Err(_) => {
                        self.runner_state = RunnerState::Undefined;
                        Err("Failed to start server thread, server is in undefined state")
                    }
                    Ok(thread_handle) => {
                        self.runner_state = RunnerState::Running(thread_handle);
                        Ok(())
                    }
                }
            }
            (RunnerState::Undefined, _) => Err("Server is in undefined state"),
            (state, None) => {
                self.runner_state = state;
                Err("Server has not been finalized")
            }
        }
    }

    pub(crate) fn stop(&mut self) {
        match self.runner_state.take() {
            RunnerState::Stopped(rx) => {
                self.runner_state = RunnerState::Stopped(rx);
            }
            RunnerState::Running(handle) => {
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
        if let RunnerState::Running(_) = self.runner_state {
            return !self.stop_event.is_set();
        }
        false
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub(crate) fn update(&self, duration: Option<f32>) {
        if self.connected.load(Ordering::Acquire) {
            let duration = duration.unwrap_or(0.0);
            let header = ServerHeader::Update(duration);
            self.sender.send(header.serialize_to_bytes());
        }
    }

    pub(crate) fn add_value(
        &mut self,
        name: &str,
        type_id: u64,
        value: Bytes,
    ) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.values.contains_key(&id) {
            return Err(format!("Value with id {} already exists", id));
        }

        let val = Value::new(
            name.to_string(),
            id,
            value,
            self.sender.clone(),
            self.connected.clone(),
            self.signals.clone(),
        );

        self.states.types.insert(id, type_id);
        self.states.values.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_static(
        &mut self,
        name: &str,
        type_id: u64,
        value: Bytes,
    ) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.static_values.contains_key(&id) {
            return Err(format!("Static value with id {} already exists", id));
        }

        let val = ValueStatic::new(id, value, self.sender.clone(), self.connected.clone());

        self.states.types.insert(id, type_id);
        self.states.static_values.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_signal(&mut self, name: &str, type_id: u64) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.signals.contains_key(&id) {
            return Err(format!("Signal with id {} already exists", id));
        }

        let val = Signal::new(name.to_string(), id, self.signals.clone());

        self.states.types.insert(id, type_id);
        self.states.signals.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_list(&mut self, name: &str, type_id: u64) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.lists.contains_key(&id) {
            return Err(format!("List with id {} already exists", id));
        }

        let val = ValueList::new(id, self.sender.clone(), self.connected.clone());

        self.states.types.insert(id, type_id);
        self.states.lists.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_map(&mut self, name: &str, type_id: u64) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.maps.contains_key(&id) {
            return Err(format!("Map with id {} already exists", id));
        }

        let val = ValueMap::new(id, self.sender.clone(), self.connected.clone());

        self.states.types.insert(id, type_id);
        self.states.maps.insert(id, val);

        Ok(id)
    }

    pub(crate) fn add_image(&mut self, name: &str) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.images.contains_key(&id) {
            return Err(format!("Image with id {} already exists", id));
        }

        let val = ValueImage::new(id, self.sender.clone(), self.connected.clone());

        self.states.types.insert(id, 42);
        self.states.images.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_graphs(&mut self, name: &str, graphs_type: GraphType) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.graphs.contains_key(&id) {
            return Err(format!("Graphs with id {} already exists", id));
        }

        let val = ValueGraphs::new(id, self.sender.clone(), graphs_type, self.connected.clone());

        self.states
            .types
            .insert(id, graphs_type.bytes_size() as u64);
        self.states.graphs.insert(id, val);
        Ok(id)
    }
}
