use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pyo3::buffer::Element;
use serde::{Deserialize, Serialize};

use egui_states_core::graphs::GraphElement;
use egui_states_core::nohash::NoHashMap;

use crate::graph::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::map::ValueMap;
use crate::sender::MessageSender;
use crate::signals::ChangedValues;
use crate::values::{Signal, UpdateValueServer, Value, ValueStatic};

pub(crate) trait SyncTrait: Sync + Send {
    fn sync(&self);
}

pub(crate) trait Acknowledge: Sync + Send {
    fn acknowledge(&self);
}

#[derive(Clone)]
pub(crate) struct PyValuesList {
    pub(crate) values: NoHashMap<u64, Arc<Value>>,
    pub(crate) static_values: NoHashMap<u64, Arc<ValueStatic>>,
    pub(crate) signals: NoHashMap<u64, Arc<Signal>>,
    pub(crate) images: NoHashMap<u64, Arc<ValueImage>>,
    pub(crate) maps: NoHashMap<u64, Arc<ValueMap>>,
    pub(crate) lists: NoHashMap<u64, Arc<ValueList>>,
    pub(crate) graphs: NoHashMap<u64, Arc<ValueGraphs>>,
}

impl PyValuesList {
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
    pub(crate) update: NoHashMap<u64, Arc<dyn UpdateValueServer>>,
    pub(crate) ack: NoHashMap<u64, Arc<dyn Acknowledge>>,
    pub(crate) sync: NoHashMap<u64, Arc<dyn SyncTrait>>,
}

impl ServerStatesList {
    fn new() -> Self {
        Self {
            update: NoHashMap::default(),
            ack: NoHashMap::default(),
            sync: NoHashMap::default(),
        }
    }

    fn shrink(&mut self) {
        self.update.shrink_to_fit();
        self.ack.shrink_to_fit();
        self.sync.shrink_to_fit();
    }
}

pub struct ServerValuesCreator {
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    signals: ChangedValues,

    version: u64,
    val: ServerStatesList,
    py_val: PyValuesList,
}

impl ServerValuesCreator {
    pub(crate) fn new(
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        signals: ChangedValues,
    ) -> Self {
        Self {
            sender,
            connected,
            signals,

            version: 0,
            val: ServerStatesList::new(),
            py_val: PyValuesList::new(),
        }
    }

    pub(crate) fn get_values(self) -> (ServerStatesList, PyValuesList, u64) {
        let Self {
            mut val,
            mut py_val,
            ..
        } = self;
        val.shrink();
        py_val.shrink();

        (val, py_val, self.version)
    }

    pub fn set_version(&mut self, version: u64) {
        self.version = version;
    }

    pub fn add_value<T>(&mut self, id: u32, value: T)
    where
        T: ToPython + FromPython + Serialize + for<'a> Deserialize<'a> + Clone + 'static,
    {
        let value = PyValue::new(
            id,
            value,
            self.sender.clone(),
            self.connected.clone(),
            self.signals.clone(),
        );

        self.py_val.values.insert(id, value.clone());
        self.val.update.insert(id, value.clone());
        self.val.sync.insert(id, value.clone());
        self.val.ack.insert(id, value);
    }

    pub fn add_static<T>(&mut self, id: u32, value: T)
    where
        T: ToPython + FromPython + Serialize + Clone + 'static,
    {
        let value = PyValueStatic::new(id, value, self.sender.clone(), self.connected.clone());

        self.py_val.static_values.insert(id, value.clone());
        self.val.sync.insert(id, value);
    }

    pub fn add_signal<T: Clone + ToPython + FromPython + for<'a> Deserialize<'a> + 'static>(
        &mut self,
        id: u32,
    ) {
        let signal = PySignal::<T>::new(id, self.signals.clone());

        self.py_val.signals.insert(id, signal.clone());
        self.val.update.insert(id, signal);
    }

    pub fn add_image(&mut self, id: u32) {
        let image = PyValueImage::new(id, self.sender.clone(), self.connected.clone());

        self.py_val.images.insert(id, image.clone());
        self.val.ack.insert(id, image.clone());
        self.val.sync.insert(id, image);
    }

    pub fn add_dict<K, V>(&mut self, id: u32)
    where
        K: ToPython + FromPython + Serialize + Eq + Hash + 'static,
        V: ToPython + FromPython + Serialize + 'static,
    {
        let dict = PyValueDict::<K, V>::new(id, self.sender.clone(), self.connected.clone());

        self.py_val.maps.insert(id, dict.clone());
        self.val.sync.insert(id, dict);
    }

    pub fn add_list<T>(&mut self, id: u32)
    where
        T: ToPython + FromPython + Serialize + Clone + 'static,
    {
        let list = PyValueList::<T>::new(id, self.sender.clone(), self.connected.clone());

        self.py_val.lists.insert(id, list.clone());
        self.val.sync.insert(id, list);
    }

    pub fn add_graphs<T: GraphElement + Element + Serialize + FromPython + ToPython + 'static>(
        &mut self,
        id: u32,
    ) {
        let graph = PyValueGraphs::<T>::new(id, self.sender.clone(), self.connected.clone());

        self.py_val.graphs.insert(id, graph.clone());
        self.val.sync.insert(id, graph);
    }
}
