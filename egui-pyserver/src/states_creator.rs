use std::hash::Hash;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use pyo3::buffer::Element;
use pyo3::{FromPyObject, ToPyObject};

use egui_pytransport::collections::CollectionItem;
use egui_pytransport::graphs::GraphElement;
use egui_pytransport::transport::WriteMessage;
use egui_pytransport::values::{ReadValue, WriteValue};
use egui_pytransport::{EnumInt, EnumStr, NoHashMap};

use crate::dict::{PyDict, ValueDict};
use crate::graphs::{PyGraph, ValueGraph};
use crate::image::ImageValue;
use crate::list::{PyListTrait, ValueList};
use crate::signals::ChangedValues;
use crate::values::{ProccesValue, PyValue, PyValueStatic};
use crate::values::{Signal, Value, ValueEnum, ValueStatic};
use crate::{Acknowledge, SyncTrait};

#[derive(Clone)]
pub(crate) struct PyValuesList {
    pub(crate) values: NoHashMap<u32, Arc<dyn PyValue>>,
    pub(crate) static_values: NoHashMap<u32, Arc<dyn PyValueStatic>>,
    pub(crate) images: NoHashMap<u32, Arc<ImageValue>>,
    pub(crate) dicts: NoHashMap<u32, Arc<dyn PyDict>>,
    pub(crate) lists: NoHashMap<u32, Arc<dyn PyListTrait>>,
    pub(crate) graphs: NoHashMap<u32, Arc<dyn PyGraph>>,
}

impl PyValuesList {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            static_values: NoHashMap::default(),
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
    pub(crate) updated: NoHashMap<u32, Arc<dyn ProccesValue>>,
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

pub struct ValuesCreator {
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
    signals: ChangedValues,

    version: u64,
    counter: u32,
    val: ValuesList,
    py_val: PyValuesList,
}

impl ValuesCreator {
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
            counter: 10, // first 10 values are reserved for special values
            val: ValuesList::new(),
            py_val: PyValuesList::new(),
        }
    }

    fn get_id(&mut self) -> u32 {
        if self.counter > 16777215 {
            panic!("id counter overflow, id is 24bit long");
        }
        let count = self.counter;
        self.counter += 1;
        count
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

    pub fn add_value<T>(&mut self, value: T) -> Arc<Value<T>>
    where
        T: ReadValue + WriteValue + ToPyObject + for<'py> FromPyObject<'py>,
    {
        let id = self.get_id();
        let value = Value::new(
            id,
            value,
            self.channel.clone(),
            self.connected.clone(),
            self.signals.clone(),
        );

        self.py_val.values.insert(id, value.clone());
        self.val.updated.insert(id, value.clone());
        self.val.sync.insert(id, value.clone());
        self.val.ack.insert(id, value.clone());

        value
    }

    pub fn add_static_value<T>(&mut self, value: T) -> Arc<ValueStatic<T>>
    where
        T: WriteValue + ToPyObject + for<'py> FromPyObject<'py> + Sync + Send + Clone + 'static,
    {
        let id = self.get_id();
        let value = ValueStatic::new(id, value, self.channel.clone(), self.connected.clone());

        self.py_val.static_values.insert(id, value.clone());
        self.val.sync.insert(id, value.clone());

        value
    }

    pub fn add_enum<T: EnumInt + EnumStr + PartialEq + 'static>(
        &mut self,
        value: T,
    ) -> Arc<ValueEnum<T>> {
        let id = self.get_id();
        let value = ValueEnum::new(
            id,
            value,
            self.channel.clone(),
            self.connected.clone(),
            self.signals.clone(),
        );

        self.py_val.values.insert(id, value.clone());
        self.val.updated.insert(id, value.clone());
        self.val.sync.insert(id, value.clone());
        self.val.ack.insert(id, value.clone());

        value
    }

    pub fn add_signal<T: WriteValue + ReadValue + Clone + ToPyObject + 'static>(
        &mut self,
    ) -> Arc<Signal<T>> {
        let id = self.get_id();
        let signal = Signal::new(id, self.signals.clone());

        self.val.updated.insert(id, signal.clone());

        signal
    }

    pub fn add_image(&mut self) -> Arc<ImageValue> {
        let id = self.get_id();
        let image = ImageValue::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.images.insert(id, image.clone());
        self.val.sync.insert(id, image.clone());

        image
    }

    pub fn add_dict<K, V>(&mut self) -> Arc<ValueDict<K, V>>
    where
        K: CollectionItem + ToPyObject + for<'py> FromPyObject<'py> + Eq + Hash + 'static,
        V: CollectionItem + ToPyObject + for<'py> FromPyObject<'py> + 'static,
    {
        let id = self.get_id();
        let dict = ValueDict::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.dicts.insert(id, dict.clone());
        self.val.sync.insert(id, dict.clone());

        dict
    }

    pub fn add_list<T>(&mut self) -> Arc<ValueList<T>>
    where
        T: CollectionItem + ToPyObject + for<'py> FromPyObject<'py> + 'static,
    {
        let id = self.get_id();
        let list = ValueList::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.lists.insert(id, list.clone());
        self.val.sync.insert(id, list.clone());

        list
    }

    pub fn add_graph<
        T: GraphElement + Element + for<'py> FromPyObject<'py> + ToPyObject + 'static,
    >(
        &mut self,
    ) -> Arc<ValueGraph<T>> {
        let id = self.get_id();
        let graph = ValueGraph::new(id, self.channel.clone(), self.connected.clone());

        self.py_val.graphs.insert(id, graph.clone());
        self.val.sync.insert(id, graph.clone());

        graph
    }
}
