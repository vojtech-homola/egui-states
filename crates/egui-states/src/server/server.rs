use std::net::SocketAddrV4;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

use bytes::Bytes;
use tokio::runtime::Builder;

use crate::data_transport::DataType;
use crate::event_async::Event;
use crate::hashing::{NoHashMap, generate_value_id};
use crate::serialization::{ServerHeader, serialize};
use crate::server::data_server::Data;
use crate::server::image_server::ValueImage;
use crate::server::map_server::ValueMap;
use crate::server::sender::{MessageReceiver, MessageSender};
use crate::server::server_core;
use crate::server::signals::SignalsManager;
use crate::server::values_server::{Signal, Value, ValueStatic, ValueTake};
use crate::server::vec_server::ValueList;

pub(crate) trait SyncTrait: Sync + Send {
    fn sync(&self) -> Result<(), ()>;
}

pub(crate) trait Acknowledge: Sync + Send {
    fn acknowledge(&self);
}

#[derive(Clone, Default)]
pub(crate) struct StatesList {
    pub(crate) values: NoHashMap<u64, Arc<Value>>,
    pub(crate) values_take: NoHashMap<u64, Arc<ValueTake>>,
    pub(crate) static_values: NoHashMap<u64, Arc<ValueStatic>>,
    pub(crate) signals: NoHashMap<u64, Arc<Signal>>,
    pub(crate) images: NoHashMap<u64, Arc<ValueImage>>,
    pub(crate) maps: NoHashMap<u64, Arc<ValueMap>>,
    pub(crate) lists: NoHashMap<u64, Arc<ValueList>>,
    pub(crate) datas: NoHashMap<u64, Arc<Data>>,
}

impl StatesList {
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

        for map in self.maps.values() {
            server_list.sync.push(map.clone());
        }

        for list in self.lists.values() {
            server_list.sync.push(list.clone());
        }

        for (id, data) in self.datas.iter() {
            server_list.sync.push(data.clone());
            server_list.ack.insert(*id, data.clone());
        }

        server_list
    }
}

#[derive(Clone, Default)]
pub(crate) struct ServerStatesList {
    pub(crate) values: NoHashMap<u64, Arc<Value>>,
    pub(crate) signals: NoHashMap<u64, Arc<Signal>>,
    pub(crate) ack: NoHashMap<u64, Arc<dyn Acknowledge>>,
    pub(crate) sync: Vec<Arc<dyn SyncTrait>>,
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

        let id = generate_value_id(&name);
        if self.states.values.contains_key(&id) {
            return Err(format!("Value with id {} already exists", id));
        }

        let val = Value::new(
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

        let id = generate_value_id(&name);
        if self.states.values_take.contains_key(&id) {
            return Err(format!("ValueTake with id {} already exists", id));
        }

        let val = ValueTake::new(
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

        let id = generate_value_id(&name);
        if self.states.static_values.contains_key(&id) {
            return Err(format!("Static value with id {} already exists", id));
        }

        let val = ValueStatic::new(
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

        let id = generate_value_id(&name);
        if self.states.signals.contains_key(&id) {
            return Err(format!("Signal with id {} already exists", id));
        }

        let val = Signal::new(name.to_string(), id, type_id, self.signals.clone());

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

        let id = generate_value_id(&name);
        if self.states.lists.contains_key(&id) {
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

        let id = generate_value_id(&name);
        if self.states.maps.contains_key(&id) {
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

        let id = generate_value_id(&name);
        if self.states.images.contains_key(&id) {
            return Err(format!("Image with id {} already exists", id));
        }

        let val = ValueImage::new(
            name.to_string(),
            id,
            self.sender.clone(),
            self.connected.clone(),
        );

        self.states.images.insert(id, val);
        Ok(id)
    }

    pub(crate) fn add_data(&mut self, name: &str, type_id: u8) -> Result<u64, String> {
        if self.states_server.is_some() {
            return Err("Cannot add new values after server has been finalized".to_string());
        }

        let id = generate_value_id(&name);
        if self.states.datas.contains_key(&id) {
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

        self.states.datas.insert(id, val);
        Ok(id)
    }
}
