use std::net::{SocketAddrV4, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

use bytes::Bytes;
use tokio::runtime::Builder;

use egui_states_core::controls::ControlServer;
use egui_states_core::event_async::Event;
use egui_states_core::graphs::GraphType;
use egui_states_core::nohash::NoHashMap;
use egui_states_core::serialization::{ServerHeader, serialize_value_vec};

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
    Running(thread::JoinHandle<MessageReceiver>, MessageSender),
    Stopped(MessageReceiver, MessageSender),
    Uninitialized,
}

impl RunnerState {
    #[inline]
    fn is_running(&self) -> bool {
        match self {
            RunnerState::Running(_, _) => true,
            RunnerState::Stopped(_, _) => false,
            RunnerState::Uninitialized => false,
        }
    }

    fn take(&mut self) -> Self {
        match std::mem::replace(self, RunnerState::Uninitialized) {
            RunnerState::Running(handle, sender) => RunnerState::Running(handle, sender),
            RunnerState::Stopped(rx, sender) => RunnerState::Stopped(rx, sender),
            RunnerState::Uninitialized => RunnerState::Uninitialized,
        }
    }

    fn check_state(&mut self) -> MessageSender {
        match self {
            RunnerState::Uninitialized => {
                let (sender, rx) = MessageSender::new();
                *self = RunnerState::Stopped(rx, sender.clone());
                sender
            }
            RunnerState::Stopped(_, sender) => sender.clone(),
            RunnerState::Running(_, sender) => sender.clone(),
        }
    }

    fn get_sender(&self, f: impl Fn(&MessageSender)) {
        match self {
            RunnerState::Uninitialized => {}
            RunnerState::Stopped(_, sender) => f(sender),
            RunnerState::Running(_, sender) => f(sender),
        }
    }
}

pub(crate) struct Server {
    connected: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    start_event: Event,
    addr: SocketAddrV4,
    states: StatesList,
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
        let start_event = Event::new();
        let enabled = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));
        let (sender, rx) = MessageSender::new();
        let signals = SignalsManager::new();

        let obj = Self {
            connected,
            enabled,
            start_event,
            addr,
            states: StatesList::default(),
            signals,
            handshake,
            // states_server: ServerStatesList::new(),
            runner_state: RunnerState::Stopped(rx, sender),
            runner_threads,
        };

        obj
    }

    pub(crate) fn finalize(&mut self) -> Option<StatesList> {
        if self.states_server.is_none() {
            return None;
        }

        let runtime = Builder::new_current_thread()
            .thread_name("Server Runtime")
            .enable_io()
            .worker_threads(3)
            .build()
            .unwrap();

        let sender = self.sender.clone();
        let rx = self.rx.take().unwrap();
        let connected = self.connected.clone();
        let enabled = self.enabled.clone();
        let mut values = self.states_server.take().unwrap();
        values.shrink();
        let signals = self.signals.clone();
        let start_event = self.start_event.clone();
        let handshake = self.handshake.clone();
        let addr = self.addr;

        let server_thread = thread::Builder::new().name("Server".to_string());
        let thread_handle = server_thread.spawn(move || {
            runtime.block_on(async move {
                server_core::run(
                    sender,
                    rx,
                    connected,
                    enabled,
                    values,
                    signals,
                    start_event,
                    addr,
                    handshake,
                )
                .await;
            });
        });

        self.states.shrink();
        Some(self.states.clone())
    }

    pub(crate) fn get_signals_manager(&self) -> SignalsManager {
        self.signals.clone()
    }

    pub(crate) fn start(&mut self) -> Result<StatesList, ()> {
        match self.runner_state {}















        let runtime = Builder::new_current_thread()
            .thread_name("Server Runtime")
            .enable_io()
            .worker_threads(3)
            .build()
            .unwrap();

        let sender = self.sender.clone();
        let rx = self.rx.take().unwrap();
        let connected = self.connected.clone();
        let enabled = self.enabled.clone();
        self.states_server.shrink();
        let values = self.states_server.clone();
        let signals = self.signals.clone();
        let start_event = self.start_event.clone();
        let handshake = self.handshake.clone();
        let addr = self.addr;

        let server_thread = thread::Builder::new().name("Server".to_string());
        let thread_handle = server_thread
            .spawn(move || {
                runtime.block_on(async move {
                    server_core::run(
                        sender, rx, connected, enabled, values, signals, addr, handshake,
                    )
                    .await
                })
            })
            .map_err(|_| ())?;

        self.thread_handle = Some(thread_handle);
        self.enabled.store(true, Ordering::Release);
        self.start_event.set();

        Ok(self.states.clone())
    }

    pub(crate) fn stop(&mut self) {
        let state = self.runner_state.take();
        match state {
            RunnerState::Stopped(rx, sender) => {
                self.runner_state = RunnerState::Stopped(rx, sender);
            }
            RunnerState::Running(handle, sender) => {
                self.enabled.store(false, Ordering::Release);
                self.connected.store(false, Ordering::Release);
                sender.close();

                // try to connect to the server to unblock the accept call
                TcpStream::connect(self.addr); // TODO: use localhost?

                match handle.join() {
                    Ok(rx) => {
                        self.runner_state = RunnerState::Stopped(rx, sender);
                    }
                    Err(_) => {
                        self.runner_state = RunnerState::Uninitialized;
                    }
                }
            }
            RunnerState::Uninitialized => {}
        }
    }

    pub(crate) fn disconnect_client(&mut self) {
        if self.connected.load(Ordering::Release) {
            self.connected.store(false, Ordering::Release);
            self.runner_state.get_sender(|sender| {
                sender.close();
            });
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        if let RunnerState::Running(_, _) = self.runner_state {
            return true;
        }
        false
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.connected.load(Relaxed)
    }

    pub(crate) fn update(&self, duration: Option<f32>) {
        if self.connected.load(Relaxed) {
            let duration = duration.unwrap_or(0.0);
            let header = ServerHeader::Control(ControlServer::Update(duration));
            self.runner_state
                .get_sender(|sender| sender.send(header.serialize_to_bytes()));
        }
    }

    pub(crate) fn add_value(&mut self, id: u64, type_id: u64, value: Bytes) -> Result<(), String> {
        if self.states.values.contains_key(&id) {
            return Err(format!("Value with id {} already exists", id));
        }

        let sender = self.runner_state.check_state();
        let val = Value::new(
            id,
            value,
            sender,
            self.connected.clone(),
            self.signals.clone(),
        );

        self.states.types.insert(id, type_id);
        self.states.values.insert(id, val);
        Ok(())
    }

    pub(crate) fn add_static(&mut self, id: u64, type_id: u64, value: Bytes) -> Result<(), String> {
        if self.states.static_values.contains_key(&id) {
            return Err(format!("Static value with id {} already exists", id));
        }
        let sender = self.runner_state.check_state();
        let val = ValueStatic::new(id, value, sender, self.connected.clone());

        self.states.types.insert(id, type_id);
        self.states.static_values.insert(id, val);
        Ok(())
    }

    pub(crate) fn add_signal(&mut self, id: u64, type_id: u64) -> Result<(), String> {
        if self.states.signals.contains_key(&id) {
            return Err(format!("Signal with id {} already exists", id));
        }

        let val = Signal::new(id, self.signals.clone());

        self.states.types.insert(id, type_id);
        self.states.signals.insert(id, val);
        Ok(())
    }

    pub(crate) fn add_list(&mut self, id: u64, type_id: u64) -> Result<(), String> {
        if self.states.lists.contains_key(&id) {
            return Err(format!("List with id {} already exists", id));
        }
        let sender = self.runner_state.check_state();
        let val = ValueList::new(id, sender, self.connected.clone());

        self.states.types.insert(id, type_id);
        self.states.lists.insert(id, val);
        Ok(())
    }

    pub(crate) fn add_map(&mut self, id: u64, type_id: u64) -> Result<(), String> {
        if self.states.maps.contains_key(&id) {
            return Err(format!("Map with id {} already exists", id));
        }
        let sender = self.runner_state.check_state();
        let val = ValueMap::new(id, sender, self.connected.clone());

        self.states.types.insert(id, type_id);
        self.states.maps.insert(id, val);

        Ok(())
    }

    pub(crate) fn add_image(&mut self, id: u64) -> Result<(), String> {
        if self.states.images.contains_key(&id) {
            return Err(format!("Image with id {} already exists", id));
        }
        let sender = self.runner_state.check_state();

        let val = ValueImage::new(id, sender, self.connected.clone());

        self.states.types.insert(id, 42);
        self.states.images.insert(id, val);
        Ok(())
    }

    pub(crate) fn add_graphs(&mut self, id: u64, graphs_type: GraphType) -> Result<(), String> {
        if self.states.graphs.contains_key(&id) {
            return Err(format!("Graphs with id {} already exists", id));
        }

        let sender = self.runner_state.check_state();
        let val = ValueGraphs::new(id, sender, graphs_type, self.connected.clone());

        self.states
            .types
            .insert(id, graphs_type.bytes_size() as u64);
        self.states.graphs.insert(id, val);
        Ok(())
    }
}
