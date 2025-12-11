use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use egui_states_core::generate_value_id;
use egui_states_core::graphs::GraphElement;
use egui_states_core::nohash::NoHashMap;
use egui_states_core::types::{GetType, ObjectType};

use crate::State;
use crate::graphs::{UpdateGraph, ValueGraphs};
use crate::image::ValueImage;
use crate::list::{UpdateList, ValueList};
use crate::map::{UpdateMap, ValueMap};
use crate::sender::MessageSender;
use crate::states_creator::{StatesBuilder, StatesCreator};
use crate::values::{Signal, UpdateValue, Value, ValueStatic};

#[derive(Clone)]
pub(crate) struct ValuesList {
    pub(crate) values: NoHashMap<u64, Arc<dyn UpdateValue>>,
    pub(crate) static_values: NoHashMap<u64, Arc<dyn UpdateValue>>,
    pub(crate) images: NoHashMap<u64, Arc<ValueImage>>,
    pub(crate) maps: NoHashMap<u64, Arc<dyn UpdateMap>>,
    pub(crate) lists: NoHashMap<u64, Arc<dyn UpdateList>>,
    pub(crate) graphs: NoHashMap<u64, Arc<dyn UpdateGraph>>,
    pub(crate) types: NoHashMap<u64, u64>,
}

impl ValuesList {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            static_values: NoHashMap::default(),
            images: NoHashMap::default(),
            maps: NoHashMap::default(),
            lists: NoHashMap::default(),
            graphs: NoHashMap::default(),
            types: NoHashMap::default(),
        }
    }

    fn shrink(&mut self) {
        self.values.shrink_to_fit();
        self.static_values.shrink_to_fit();
        self.images.shrink_to_fit();
        self.maps.shrink_to_fit();
        self.lists.shrink_to_fit();
        self.graphs.shrink_to_fit();
        self.types.shrink_to_fit();
    }
}

pub struct StatesCreatorClient {
    val: ValuesList,
    sender: MessageSender,
}

impl StatesCreatorClient {
    pub(crate) fn new(sender: MessageSender) -> Self {
        Self {
            val: ValuesList::new(),
            sender,
        }
    }

    pub(crate) fn get_values(self) -> ValuesList {
        let mut val = self.val;
        val.shrink();
        val
    }
}

impl StatesCreator for StatesCreatorClient {
    type Builder = StatesBuilderClient;

    fn builder(&mut self, _state_name: &'static str, parent: &String) -> Self::Builder {
        StatesBuilderClient {
            parent: parent.clone(),
            sender: self.sender.clone(),
            val: ValuesList::new(),
        }
    }

    fn add_states(&mut self, builder: StatesBuilderClient) {
        self.val.values.extend(builder.val.values);
        self.val.static_values.extend(builder.val.static_values);
        self.val.images.extend(builder.val.images);
        self.val.maps.extend(builder.val.maps);
        self.val.lists.extend(builder.val.lists);
        self.val.graphs.extend(builder.val.graphs);
        self.val.types.extend(builder.val.types);
    }

    fn add_substate<S: State>(&mut self, parent: &str, name: &str) -> S {
        S::new(self, format!("{}.{}", parent, name))
    }
}

pub struct StatesBuilderClient {
    parent: String,
    sender: MessageSender,
    val: ValuesList,
}

impl StatesBuilder for StatesBuilderClient {
    fn add_value<T>(&mut self, name: &str, value: T) -> Arc<Value<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + GetType + Send + Sync + Clone + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = Value::new(id, value, self.sender.clone());

        self.val.values.insert(id, value.clone());
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn add_static<T>(&mut self, name: &str, value: T) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + GetType + Clone + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueStatic::new(id, value);

        self.val.static_values.insert(id, value.clone());
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn add_image(&mut self, name: &str) -> Arc<ValueImage> {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueImage::new(id, self.sender.clone());

        self.val.images.insert(id, value.clone());
        self.val.types.insert(id, 42);
        value
    }

    fn add_signal<T>(&mut self, name: &str) -> Arc<Signal<T>>
    where
        T: Serialize + GetType + Clone + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let signal = Signal::new(id, self.sender.clone());
        self.val.types.insert(id, T::get_type().get_hash());

        signal
    }

    fn add_map<K, V>(&mut self, name: &str) -> Arc<ValueMap<K, V>>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueMap::new(id);

        self.val.maps.insert(id, value.clone());
        self.val.types.insert(
            id,
            ObjectType::Map(Box::new(K::get_type()), Box::new(V::get_type())).get_hash(),
        );
        value
    }

    fn add_list<T>(&mut self, name: &str) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueList::new(id);

        self.val.lists.insert(id, value.clone());
        self.val
            .types
            .insert(id, ObjectType::Vec(Box::new(T::get_type())).get_hash());
        value
    }

    fn add_graphs<T>(&mut self, name: &str) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GraphElement + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueGraphs::new(id);

        self.val.graphs.insert(id, value.clone());
        self.val.types.insert(id, T::bytes_size() as u64);
        value
    }
}
