use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use egui_states_core::graphs::GraphElement;
use egui_states_core::nohash::NoHashMap;
use egui_states_core::types::GetType;

use crate::graphs::{UpdateGraph, ValueGraphs};
use crate::image::ValueImage;
use crate::list::{UpdateList, ValueList};
use crate::map::{UpdateMap, ValueMap};
use crate::sender::MessageSender;
use crate::values::{Signal, UpdateValue, Value, ValueStatic};
use crate::{GetInitValue, GetTypeInfo, State};

pub trait ValuesCreator {
    fn add_value<T>(&mut self, state: &'static str, name: &'static str, value: T) -> Arc<Value<T>>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetTypeInfo
            + GetInitValue
            + GetType
            + Send
            + Sync
            + Clone
            + 'static;

    fn add_static<T>(
        &mut self,
        state: &'static str,
        name: &'static str,
        value: T,
    ) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetInitValue
            + GetTypeInfo
            + GetType
            + Clone
            + Send
            + Sync
            + 'static;

    fn add_image(&mut self, state: &'static str, name: &'static str) -> Arc<ValueImage>;

    fn add_signal<T>(&mut self, state: &'static str, name: &'static str) -> Arc<Signal<T>>
    where
        T: Serialize + GetType + Clone + Send + Sync + GetTypeInfo + 'static;

    fn add_dict<K, V>(&mut self, state: &'static str, name: &'static str) -> Arc<ValueMap<K, V>>
    where
        K: Hash
            + Eq
            + Clone
            + for<'a> Deserialize<'a>
            + GetType
            + Send
            + GetTypeInfo
            + Sync
            + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + GetTypeInfo + GetType + Sync + 'static;

    fn add_list<T>(&mut self, state: &'static str, name: &'static str) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + GetType + Send + Sync + GetTypeInfo + 'static;

    fn add_graphs<T>(&mut self, state: &'static str, name: &'static str) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GraphElement + GetTypeInfo + 'static;

    fn add_substate<S: State>(&mut self, state: &'static str, name: &'static str) -> S;
}

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

pub struct ClientValuesCreator {
    counter: u64,
    val: ValuesList,
    states_hash: u64,
    sender: MessageSender,
}

impl ClientValuesCreator {
    pub(crate) fn new(sender: MessageSender) -> Self {
        Self {
            counter: 9, // first 10 values are reserved for special values
            val: ValuesList::new(),
            states_hash: 0,
            sender,
        }
    }

    fn get_id(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    pub(crate) fn get_values(self) -> (ValuesList, u64) {
        let mut val = self.val;
        val.shrink();
        (val, self.states_hash)
    }
}

impl ValuesCreator for ClientValuesCreator {
    fn add_value<T>(&mut self, _: &'static str, _: &'static str, value: T) -> Arc<Value<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + GetType + Send + Sync + Clone + 'static,
    {
        let id = self.get_id();
        let value = Value::new(id, value, self.sender.clone());

        self.val.values.insert(id, value.clone());
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn add_static<T>(&mut self, _: &'static str, _: &'static str, value: T) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + GetType + Clone + Send + Sync + 'static,
    {
        let id = self.get_id();
        let value = ValueStatic::new(id, value);

        self.val.static_values.insert(id, value.clone());
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn add_image(&mut self, _: &'static str, _: &'static str) -> Arc<ValueImage> {
        let id = self.get_id();
        let value = ValueImage::new(id, self.sender.clone());

        self.val.images.insert(id, value.clone());
        self.val.types.insert(id, 42);
        value
    }

    fn add_signal<T>(&mut self, _: &'static str, _: &'static str) -> Arc<Signal<T>>
    where
        T: Serialize + GetType + Clone + Send + Sync + 'static,
    {
        let id = self.get_id();
        let signal = Signal::new(id, self.sender.clone());
        self.val.types.insert(id, T::get_type().get_hash());

        signal
    }

    fn add_dict<K, V>(&mut self, _: &'static str, _: &'static str) -> Arc<ValueMap<K, V>>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
    {
        let id = self.get_id();
        let value = ValueMap::new(id);

        self.val.maps.insert(id, value.clone());
        self.val
            .types
            .insert(id, V::get_type().get_hash_add(K::get_type()));
        value
    }

    fn add_list<T>(&mut self, _: &'static str, _: &'static str) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
    {
        let id = self.get_id();
        let value = ValueList::new(id);

        self.val.lists.insert(id, value.clone());
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn add_graphs<T>(&mut self, _: &'static str, _: &'static str) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GraphElement + 'static,
    {
        let id = self.get_id();
        let value = ValueGraphs::new(id);

        self.val.graphs.insert(id, value.clone());
        self.val.types.insert(id, T::bytes_size() as u64);
        value
    }

    fn add_substate<S: State>(&mut self, _: &'static str, _: &'static str) -> S {
        S::new(self)
    }
}
