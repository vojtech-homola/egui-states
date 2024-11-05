use std::hash::Hash;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use egui_pysync::collections::CollectionItem;
use egui_pysync::graphs::GraphElement;
use egui_pysync::transport::WriteMessage;
use egui_pysync::values::{ReadValue, WriteValue};
use egui_pysync::{EnumInt, NoHashMap};

use crate::dict::{DictUpdate, ValueDict};
use crate::graphs::{GraphUpdate, ValueGraphs};
use crate::image::{ImageUpdate, ValueImage};
use crate::list::{ListUpdate, ValueList};
use crate::values::{Signal, Value, ValueEnum, ValueStatic, ValueUpdate};

#[derive(Clone)]
pub(crate) struct ValuesList {
    pub(crate) values: NoHashMap<u32, Arc<dyn ValueUpdate>>,
    pub(crate) static_values: NoHashMap<u32, Arc<dyn ValueUpdate>>,
    pub(crate) images: NoHashMap<u32, Arc<dyn ImageUpdate>>,
    pub(crate) dicts: NoHashMap<u32, Arc<dyn DictUpdate>>,
    pub(crate) lists: NoHashMap<u32, Arc<dyn ListUpdate>>,
    pub(crate) graphs: NoHashMap<u32, Arc<dyn GraphUpdate>>,
}

impl ValuesList {
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

pub struct ValuesCreator {
    counter: u32,
    val: ValuesList,
    version: u64,
    channel: Sender<WriteMessage>,
}

impl ValuesCreator {
    pub(crate) fn new(channel: Sender<WriteMessage>) -> Self {
        Self {
            counter: 10, // first 10 values are reserved for special values
            val: ValuesList::new(),
            version: 0,
            channel,
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

    pub(crate) fn get_values(self) -> (ValuesList, u64) {
        let mut val = self.val;
        val.shrink();
        (val, self.version)
    }

    pub fn set_version(&mut self, version: u64) {
        self.version = version;
    }

    pub fn add_value<T>(&mut self, value: T) -> Arc<Value<T>>
    where
        T: WriteValue + ReadValue + 'static,
    {
        let id = self.get_id();
        let value = Value::new(id, value, self.channel.clone());

        self.val.values.insert(id, value.clone());
        value
    }

    pub fn add_static<T>(&mut self, value: T) -> Arc<ValueStatic<T>>
    where
        T: ReadValue + 'static,
    {
        let id = self.get_id();
        let value = ValueStatic::new(id, value);

        self.val.static_values.insert(id, value.clone());
        value
    }

    pub fn add_image(&mut self) -> Arc<ValueImage> {
        let id = self.get_id();
        let value = ValueImage::new(id);

        self.val.images.insert(id, value.clone());
        value
    }

    pub fn add_enum<T: EnumInt + 'static>(&mut self, value: T) -> Arc<ValueEnum<T>> {
        let id = self.get_id();
        let value = ValueEnum::new(id, value, self.channel.clone());

        self.val.values.insert(id, value.clone());
        value
    }

    pub fn add_signal<T: WriteValue + Clone + 'static>(&mut self) -> Arc<Signal<T>> {
        let id = self.get_id();
        let signal = Signal::new(id, self.channel.clone());

        signal
    }

    pub fn add_dict<K, V>(&mut self) -> Arc<ValueDict<K, V>>
    where
        K: CollectionItem + Hash + Eq,
        V: CollectionItem,
    {
        let id = self.get_id();
        let value = ValueDict::new(id);

        self.val.dicts.insert(id, value.clone());
        value
    }

    pub fn add_list<T>(&mut self) -> Arc<ValueList<T>>
    where
        T: CollectionItem,
    {
        let id = self.get_id();
        let value = ValueList::new(id);

        self.val.lists.insert(id, value.clone());
        value
    }

    pub fn add_graphs<T: GraphElement + 'static>(&mut self) -> Arc<ValueGraphs<T>> {
        let id = self.get_id();
        let value = ValueGraphs::new(id);

        self.val.graphs.insert(id, value.clone());
        value
    }
}
