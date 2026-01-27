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
use crate::states_creator::StatesCreator;
use crate::values::{GetQueueType, Signal, Static, StaticAtomic, UpdateValue, Value, ValueAtomic};
use crate::values_atomic::Atomic;

#[derive(Clone)]
pub(crate) struct ValuesList {
    pub(crate) values: NoHashMap<u64, Arc<dyn UpdateValue>>,
    pub(crate) static_values: NoHashMap<u64, Arc<dyn UpdateValue>>,
    pub(crate) images: NoHashMap<u64, ValueImage>,
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
    parent: String,
}

impl StatesCreatorClient {
    pub(crate) fn new(sender: MessageSender, parent: String) -> Self {
        Self {
            val: ValuesList::new(),
            sender,
            parent,
        }
    }

    pub(crate) fn get_values(self) -> ValuesList {
        let mut val = self.val;
        val.shrink();
        val
    }
}

impl StatesCreator for StatesCreatorClient {
    fn substate<S: State>(&mut self, name: &str) -> S {
        let parent = format!("{}.{}", self.parent, name);
        let mut creator = StatesCreatorClient::new(self.sender.clone(), parent);
        let substate = S::new(&mut creator);

        self.val.values.extend(creator.val.values);
        self.val.static_values.extend(creator.val.static_values);
        self.val.images.extend(creator.val.images);
        self.val.maps.extend(creator.val.maps);
        self.val.lists.extend(creator.val.lists);
        self.val.graphs.extend(creator.val.graphs);
        self.val.types.extend(creator.val.types);

        substate
    }

    fn value<T, Q>(&mut self, name: &str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + GetType + Send + Sync + Clone + 'static,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = Value::new(id, value, self.sender.clone());

        self.val.values.insert(id, Arc::new(value.clone()));
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn atomic<T, Q>(&mut self, name: &str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + GetType + Send + Sync + Clone + Atomic + 'static,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueAtomic::new(id, value, self.sender.clone());

        self.val.values.insert(id, Arc::new(value.clone()));
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn add_static<T>(&mut self, name: &str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + GetType + Clone + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = Static::new(id, value);

        self.val.static_values.insert(id, Arc::new(value.clone()));
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn static_atomic<T>(
        &mut self,
        name: &'static str,
        value: T,
    ) -> StaticAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetType
            + Clone
            + Send
            + Sync
            + crate::GetInitValue
            + Atomic
            + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = StaticAtomic::new(id, value);

        self.val.static_values.insert(id, Arc::new(value.clone()));
        self.val.types.insert(id, T::get_type().get_hash());
        value
    }

    fn image(&mut self, name: &str) -> ValueImage {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueImage::new(id, self.sender.clone());

        self.val.images.insert(id, value.clone());
        self.val.types.insert(id, 42);
        value
    }

    fn signal<T, Q>(&mut self, name: &str) -> Signal<T, Q>
    where
        T: Serialize + GetType + Clone + Send + Sync + 'static,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let signal = Signal::new(id, self.sender.clone());
        self.val.types.insert(id, T::get_type().get_hash());

        signal
    }

    fn map<K, V>(&mut self, name: &str) -> ValueMap<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueMap::new(id);

        self.val.maps.insert(id, Arc::new(value.clone()));
        self.val.types.insert(
            id,
            ObjectType::Map(Box::new(K::get_type()), Box::new(V::get_type())).get_hash(),
        );
        value
    }

    fn list<T>(&mut self, name: &str) -> ValueList<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueList::new(id);

        self.val.lists.insert(id, Arc::new(value.clone()));
        self.val
            .types
            .insert(id, ObjectType::Vec(Box::new(T::get_type())).get_hash());
        value
    }

    fn graphs<T>(&mut self, name: &str) -> ValueGraphs<T>
    where
        T: for<'a> Deserialize<'a> + GraphElement + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueGraphs::new(id);

        self.val.graphs.insert(id, Arc::new(value.clone()));
        self.val.types.insert(id, T::bytes_size() as u64);
        value
    }
}
