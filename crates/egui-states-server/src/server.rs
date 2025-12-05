use std::net::{SocketAddrV4, TcpStream};
use std::sync::{Arc, atomic};
use std::thread;

use bytes::Bytes;
use tokio::runtime::Builder;

use egui_states_core::controls::ControlServer;
use egui_states_core::event_async::Event;
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

#[derive(Clone)]
pub(crate) struct StatesList {
    pub(crate) values: NoHashMap<u64, Arc<Value>>,
    pub(crate) static_values: NoHashMap<u64, Arc<ValueStatic>>,
    pub(crate) signals: NoHashMap<u64, Arc<Signal>>,
    pub(crate) images: NoHashMap<u64, Arc<ValueImage>>,
    pub(crate) maps: NoHashMap<u64, Arc<ValueMap>>,
    pub(crate) lists: NoHashMap<u64, Arc<ValueList>>,
    pub(crate) graphs: NoHashMap<u64, Arc<ValueGraphs>>,
}

impl StatesList {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            static_values: NoHashMap::default(),
            signals: NoHashMap::default(),
            images: NoHashMap::default(),
            maps: NoHashMap::default(),
            lists: NoHashMap::default(),
            graphs: NoHashMap::default(),
        }
    }

    fn shrink(&mut self) {
        self.values.shrink_to_fit();
        self.static_values.shrink_to_fit();
        self.images.shrink_to_fit();
        self.maps.shrink_to_fit();
        self.lists.shrink_to_fit();
        self.graphs.shrink_to_fit();
    }
}

#[derive(Clone)]
pub(crate) struct ServerStatesList {
    pub(crate) values: NoHashMap<u64, Arc<Value>>,
    pub(crate) signals: NoHashMap<u64, Arc<Signal>>,
    pub(crate) ack: NoHashMap<u64, Arc<dyn Acknowledge>>,
    pub(crate) enable: NoHashMap<u64, Arc<dyn EnableTrait>>,
    pub(crate) sync: Vec<Arc<dyn SyncTrait>>,
    pub(crate) types: NoHashMap<u64, u64>,
}

impl ServerStatesList {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            signals: NoHashMap::default(),
            ack: NoHashMap::default(),
            enable: NoHashMap::default(),
            sync: Vec::new(),
            types: NoHashMap::default(),
        }
    }

    fn shrink(&mut self) {
        self.values.shrink_to_fit();
        self.signals.shrink_to_fit();
        self.ack.shrink_to_fit();
        self.enable.shrink_to_fit();
        self.sync.shrink_to_fit();
        self.types.shrink_to_fit();
    }
}

pub(crate) struct Server {
    connected: Arc<atomic::AtomicBool>,
    enabled: Arc<atomic::AtomicBool>,
    sender: MessageSender,
    start_event: Event,
    addr: SocketAddrV4,
    states: StatesList,
    signals: SignalsManager,
    handshake: Option<Vec<u64>>,

    states_server: Option<ServerStatesList>,
    rx: Option<MessageReceiver>,
}

impl Server {
    pub(crate) fn new(addr: SocketAddrV4, handshake: Option<Vec<u64>>) -> Self {
        let start_event = Event::new();
        let enabled = Arc::new(atomic::AtomicBool::new(false));
        let connected = Arc::new(atomic::AtomicBool::new(false));
        let (sender, rx) = MessageSender::new();
        let signals = SignalsManager::new();

        let obj = Self {
            connected,
            enabled,
            sender,
            start_event,
            addr,
            states: StatesList::new(),
            signals,
            handshake,
            states_server: Some(ServerStatesList::new()),
            rx: Some(rx),
        };

        obj
    }

    pub(crate) fn initialize(&mut self) -> Option<StatesList> {
        if self.states_server.is_none() {
            return None;
        }

        let runtime = Builder::new_current_thread()
            .thread_name("Server Runtime")
            .enable_io()
            .worker_threads(2)
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
        let _ = server_thread.spawn(move || {
            runtime.block_on(async move {
                server_core::start(
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

    pub(crate) fn start(&mut self) {
        if self.enabled.load(atomic::Ordering::Relaxed) || self.states_server.is_some() {
            return;
        }

        self.enabled.store(true, atomic::Ordering::Relaxed);
        self.start_event.set();
    }

    pub(crate) fn stop(&mut self) {
        if !self.enabled.load(atomic::Ordering::Relaxed) || self.states_server.is_some() {
            return;
        }

        self.start_event.clear();
        self.enabled.store(false, atomic::Ordering::Relaxed);
        self.disconnect_client();

        // try to connect to the server to unblock the accept call
        let _ = TcpStream::connect(self.addr); // TODO: use localhost?
    }

    pub(crate) fn disconnect_client(&mut self) {
        if self.states_server.is_some() {
            return;
        }

        if self.connected.load(atomic::Ordering::Relaxed) {
            self.connected.store(false, atomic::Ordering::Relaxed);
            self.sender.close();
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        if self.states_server.is_some() {
            return false;
        }

        self.enabled.load(atomic::Ordering::Relaxed)
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.connected.load(atomic::Ordering::Relaxed)
    }

    pub(crate) fn update(&self, duration: Option<f32>) {
        if self.states_server.is_some() {
            return;
        }

        let duration = duration.unwrap_or(0.0);
        let header = ServerHeader::Control(ControlServer::Update(duration));
        self.sender.send(header.serialize_to_bytes());
    }

    pub(crate) fn add_value(&mut self, id: u64, type_id: u64, value: Bytes) -> Result<(), String> {
        if let Some(states) = self.states_server.as_mut() {
            if self.states.values.contains_key(&id) {
                return Err(format!("Value with id {} already exists", id));
            }

            let val = Value::new(
                id,
                value,
                self.sender.clone(),
                self.connected.clone(),
                self.signals.clone(),
            );

            states.types.insert(id, type_id);
            states.values.insert(id, val.clone());
            states.sync.push(val.clone());
            states.ack.insert(id, val.clone());
            states.enable.insert(id, val.clone());

            self.states.values.insert(id, val);
            return Ok(());
        }

        Err(format!(
            "Cannot add value with id {}: server not initialized",
            id
        ))
    }

    pub(crate) fn add_static(&mut self, id: u64, type_id: u64, value: Bytes) -> Result<(), String> {
        if let Some(states) = self.states_server.as_mut() {
            let val = ValueStatic::new(id, value, self.sender.clone(), self.connected.clone());

            states.types.insert(id, type_id);
            states.enable.insert(id, val.clone());
            states.sync.push(val.clone());

            self.states.static_values.insert(id, val);
            return Ok(());
        }

        Err(format!(
            "Cannot add static value with id {}: server not initialized",
            id
        ))
    }

    pub(crate) fn add_signal(&mut self, id: u64, type_id: u64) -> Result<(), String> {
        if let Some(states) = self.states_server.as_mut() {
            let val = Signal::new(id, self.signals.clone());

            states.types.insert(id, type_id);
            states.signals.insert(id, val.clone());
            states.enable.insert(id, val.clone());

            self.states.signals.insert(id, val);
            return Ok(());
        }

        Err(format!(
            "Cannot add signal with id {}: server not initialized",
            id
        ))
    }

    pub(crate) fn add_list(&mut self, id: u64, type_id: u64) -> Result<(), String> {
        if let Some(states) = self.states_server.as_mut() {
            let val = ValueList::new(id, self.sender.clone(), self.connected.clone());

            states.types.insert(id, type_id);
            states.enable.insert(id, val.clone());
            states.sync.push(val.clone());

            self.states.lists.insert(id, val);
            return Ok(());
        }

        Err(format!(
            "Cannot add list with id {}: server not initialized",
            id
        ))
    }

    pub(crate) fn add_map(&mut self, id: u64, type_id: u64) -> Result<(), String> {
        if let Some(states) = self.states_server.as_mut() {
            let val = ValueMap::new(id, self.sender.clone(), self.connected.clone());

            states.types.insert(id, type_id);
            states.enable.insert(id, val.clone());
            states.sync.push(val.clone());

            self.states.maps.insert(id, val);
            return Ok(());
        }

        Err(format!(
            "Cannot add map with id {}: server not initialized",
            id
        ))
    }

    pub(crate) fn add_image(&mut self, id: u64) -> Result<(), String> {
        if let Some(states) = self.states_server.as_mut() {
            let val = ValueImage::new(id, self.sender.clone(), self.connected.clone());

            states.types.insert(id, 42);
            states.enable.insert(id, val.clone());
            states.sync.push(val.clone());

            self.states.images.insert(id, val);
            return Ok(());
        }

        Err(format!(
            "Cannot add image with id {}: server not initialized",
            id
        ))
    }

    pub(crate) fn add_graphs(&mut self, id: u64, graphs_type: GraphType) -> Result<(), String> {
        if let Some(states) = self.states_server.as_mut() {
            let val =
                ValueGraphs::new(id, self.sender.clone(), graphs_type, self.connected.clone());

            states.types.insert(id, graphs_type.bytes_size() as u64);
            states.enable.insert(id, val.clone());
            states.sync.push(val.clone());

            self.states.graphs.insert(id, val);
            return Ok(());
        }

        Err(format!(
            "Cannot add graphs with id {}: server not initialized",
            id
        ))
    }
}
