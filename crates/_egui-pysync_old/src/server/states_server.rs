use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use pyo3::buffer::Element;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

use crate::NoHashMap;
use crate::server::python_convert::ToPython;
use crate::pyvalues::pydict::{PyDictTrait, PyValueDict};
use crate::pyvalues::pygraphs::{PyGraphTrait, PyValueGraphs};
use crate::pyvalues::pyimage::PyValueImage;
use crate::pyvalues::pylist::{PyListTrait, PyValueList};
use crate::pyvalues::pyvalues::{PySignal, PyValue, PyValueStatic};
use crate::pyvalues::pyvalues::{
    PySignalTrait, PyValueStaticTrait, PyValueTrait, UpdateValueServer,
};
use crate::server::server::{Acknowledge, SyncTrait};
use crate::server::signals::ChangedValues;
use crate::transport::WriteMessage;
use crate::values_common::GraphElement;

#[derive(Clone)]
pub(crate) struct PyValuesList {
    pub(crate) values: NoHashMap<u32, Arc<dyn PyValueTrait>>,
    pub(crate) static_values: NoHashMap<u32, Arc<dyn PyValueStaticTrait>>,
    pub(crate) signals: NoHashMap<u32, Arc<dyn PySignalTrait>>,
    pub(crate) images: NoHashMap<u32, Arc<PyValueImage>>,
    pub(crate) dicts: NoHashMap<u32, Arc<dyn PyDictTrait>>,
    pub(crate) lists: NoHashMap<u32, Arc<dyn PyListTrait>>,
    pub(crate) graphs: NoHashMap<u32, Arc<dyn PyGraphTrait>>,
}

impl PyValuesList {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            static_values: NoHashMap::default(),
            signals: NoHashMap::default(),
            images: NoHashMap::default(),
            dicts: NoHashMap::default(),
            lists: NoHashMap::default(),
            graphs: NoHashMap::default(),
        }
    }

    fn shrink(&mut self) {
        self.values.shrink_to_fit();
        self.static_values.shrink_to_fit();
        self.images.shrink_to_fit();
        self.dicts.shrink_to_fit();
        self.lists.shrink_to_fit();
        self.graphs.shrink_to_fit();
    }
}

#[derive(Clone)]
pub(crate) struct ValuesList {
    pub(crate) updated: NoHashMap<u32, Arc<dyn UpdateValueServer>>,
    pub(crate) ack: NoHashMap<u32, Arc<dyn Acknowledge>>,
    pub(crate) sync: NoHashMap<u32, Arc<dyn SyncTrait>>,
}

impl ValuesList {
    fn new() -> Self {
        Self {
            updated: NoHashMap::default(),
            ack: NoHashMap::default(),
            sync: NoHashMap::default(),
        }
    }

    fn shrink(&mut self) {
        self.updated.shrink_to_fit();
        self.ack.shrink_to_fit();
        self.sync.shrink_to_fit();
    }
}

pub struct ServerValuesCreator {
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
    signals: ChangedValues,

    version: u64,
    counter: u32,
    val: ValuesList,
    py_val: PyValuesList,
}

impl ServerValuesCreator {
    pub(crate) fn new(
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
        signals: ChangedValues,
    ) -> Self {
        Self {
            channel,
            connected,
            signals,

            version: 0,
            counter: 9, // first 10 values are reserved for special values
            val: ValuesList::new(),
            py_val: PyValuesList::new(),
        }
    }

    fn get_id(&mut self) -> u32 {
        if self.counter > 16777215 {
            panic!("id counter overflow, id is 24bit long");
        }
        self.counter += 1;
        self.counter
    }

    pub(crate) fn get_values(self) -> (ValuesList, PyValuesList, u64) {
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

    pub fn add_value<T>(&mut self, value: T)
    where
        T: ToPython
            + for<'py> FromPyObject<'py>
            + Serialize
            + for<'a> Deserialize<'a>
            + Clone
            + 'static,
    {
        let id = self.get_id();
        let value = PyValue::new(
            id,
            value,
            self.channel.clone(),
            self.connected.clone(),
            self.signals.clone(),
        );

        self.py_val.values.insert(id, value.clone());
        self.val.updated.insert(id, value.clone());
        self.val.sync.insert(id, value.clone());
        self.val.ack.insert(id, value);
    }

    pub fn add_static<T>(&mut self, value: T)
    where
        T: ToPython + for<'py> FromPyObject<'py> + Serialize + Clone + 'static,
    {
        let id = self.get_id();
        let value = PyValueStatic::new(id, value, self.channel.clone(), self.connected.clone());

        self.py_val.static_values.insert(id, value.clone());
        self.val.sync.insert(id, value);
    }

    pub fn add_signal<
        T: Clone + ToPython + for<'py> FromPyObject<'py> + for<'a> Deserialize<'a> + 'static,
    >(
        &mut self,
    ) {
        let id = self.get_id();
        let signal = PySignal::<T>::new(id, self.signals.clone());

        self.py_val.signals.insert(id, signal.clone());
        self.val.updated.insert(id, signal);
    }

    pub fn add_image(&mut self) {
        let id = self.get_id();
        let image = PyValueImage::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.images.insert(id, image.clone());
        self.val.sync.insert(id, image);
    }

    pub fn add_dict<K, V>(&mut self)
    where
        K: ToPython + for<'py> FromPyObject<'py> + Serialize + Eq + Hash + 'static,
        V: ToPython + for<'py> FromPyObject<'py> + Serialize + 'static,
    {
        let id = self.get_id();
        let dict = PyValueDict::<K, V>::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.dicts.insert(id, dict.clone());
        self.val.sync.insert(id, dict);
    }

    pub fn add_list<T>(&mut self)
    where
        T: ToPython + for<'py> FromPyObject<'py> + Serialize + Clone + 'static,
    {
        let id = self.get_id();
        let list = PyValueList::<T>::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.lists.insert(id, list.clone());
        self.val.sync.insert(id, list);
    }

    pub fn add_graphs<
        T: GraphElement + Element + Serialize + for<'py> FromPyObject<'py> + ToPython + 'static,
    >(
        &mut self,
    ) {
        let id = self.get_id();
        let graph = PyValueGraphs::<T>::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.graphs.insert(id, graph.clone());
        self.val.sync.insert(id, graph);
    }
}
