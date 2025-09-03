use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use egui_states_core::graphs::GraphElement;
use egui_states_core::nohash::NoHashMap;

use crate::UpdateValue;
use crate::channel::ChannelMessage;
use crate::dict::ValueDict;
use crate::graphs::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::values::{Signal, Value, ValueStatic};

#[derive(Clone)]
pub(crate) struct ValuesList {
    pub(crate) values: NoHashMap<u32, Arc<dyn UpdateValue>>,
    pub(crate) static_values: NoHashMap<u32, Arc<dyn UpdateValue>>,
    pub(crate) images: NoHashMap<u32, Arc<dyn UpdateValue>>,
    pub(crate) dicts: NoHashMap<u32, Arc<dyn UpdateValue>>,
    pub(crate) lists: NoHashMap<u32, Arc<dyn UpdateValue>>,
    pub(crate) graphs: NoHashMap<u32, Arc<dyn UpdateValue>>,
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
    channel: Arc<dyn ChannelMessage>,
}

impl ValuesCreator {
    pub(crate) fn new(channel: Arc<dyn ChannelMessage>) -> Self {
        Self {
            counter: 9, // first 10 values are reserved for special values
            val: ValuesList::new(),
            version: 0,
            channel,
        }
    }

    fn get_id(&mut self) -> u32 {
        if self.counter > 16777215 {
            panic!("id counter overflow, id is 24bit long");
        }
        self.counter += 1;
        self.counter
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
        T: for<'a> Deserialize<'a> + Serialize + Send + Sync + Clone + 'static,
    {
        let id = self.get_id();
        let value = Value::new(id, value, self.channel.clone());

        self.val.values.insert(id, value.clone());
        value
    }

    pub fn add_static<T>(&mut self, value: T) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Send + Sync + 'static,
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

    pub fn add_signal<T>(&mut self) -> Arc<Signal<T>>
    where
        T: Serialize + Clone + Send + Sync + 'static,
    {
        let id = self.get_id();
        let signal = Signal::new(id, self.channel.clone());

        signal
    }

    pub fn add_dict<K, V>(&mut self) -> Arc<ValueDict<K, V>>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + 'static,
    {
        let id = self.get_id();
        let value = ValueDict::new(id);

        self.val.dicts.insert(id, value.clone());
        value
    }

    pub fn add_list<T>(&mut self) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + 'static,
    {
        let id = self.get_id();
        let value = ValueList::new(id);

        self.val.lists.insert(id, value.clone());
        value
    }

    pub fn add_graphs<T>(&mut self) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GraphElement + 'static,
    {
        let id = self.get_id();
        let value = ValueGraphs::new(id);

        self.val.graphs.insert(id, value.clone());
        value
    }
}
