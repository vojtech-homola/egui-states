use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::State;
use crate::client::atomics::{Atomic, AtomicStatic};
use crate::client::data::{Data, UpdateData, private::GetDataType};
use crate::client::image::ValueImage;
use crate::client::messages::MessageSender;
use crate::client::value_map::{UpdateMap, ValueMap};
use crate::client::value_vec::{UpdateList, ValueVec};
use crate::client::values::{
    GetQueueType, Signal, Static, StaticAtomic, UpdateValue, UpdateValueTake, Value, ValueAtomic,
    ValueTake,
};
use crate::hashing::{NoHashMap, generate_value_id};
use crate::transport::Transportable;

pub trait StatesCreator {
    fn substate<S: State>(&mut self, name: &str) -> S;

    fn value<T, Q>(&mut self, name: &'static str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Transportable + Send + Sync + Clone + 'static,
        Q: GetQueueType;

    fn value_take<T>(&mut self, name: &'static str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Transportable + Send + Sync + 'static;

    fn atomic<T, Q>(&mut self, name: &'static str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Transportable
            + Send
            + Sync
            + Clone
            + Atomic
            + 'static,
        Q: GetQueueType;

    fn add_static<T>(&mut self, name: &'static str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Transportable + Clone + Send + Sync + 'static;

    fn static_atomic<T>(&mut self, name: &'static str, value: T) -> StaticAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Transportable
            + Clone
            + Send
            + Sync
            + AtomicStatic
            + 'static;

    fn signal<T, Q>(&mut self, name: &'static str) -> Signal<T, Q>
    where
        T: Serialize + Transportable + Clone + Send + Sync + 'static,
        Q: GetQueueType;

    fn image(&mut self, name: &'static str) -> ValueImage;

    fn map<K, V>(&mut self, name: &'static str) -> ValueMap<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + Transportable + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + Transportable + 'static;

    fn vec<T>(&mut self, name: &'static str) -> ValueVec<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + Transportable + 'static;

    // #[allow(private_bounds)]
    fn data<T>(&mut self, name: &'static str) -> Data<T>
    where
        T: GetDataType + Send + Sync + 'static;
}

#[derive(Clone)]
pub(crate) struct ValuesList {
    pub(crate) values: NoHashMap<u64, Arc<dyn UpdateValue>>,
    pub(crate) values_take: NoHashMap<u64, Arc<dyn UpdateValueTake>>,
    pub(crate) static_values: NoHashMap<u64, Arc<dyn UpdateValue>>,
    pub(crate) datas: NoHashMap<u64, Arc<dyn UpdateData>>,
    pub(crate) images: NoHashMap<u64, ValueImage>,
    pub(crate) maps: NoHashMap<u64, Arc<dyn UpdateMap>>,
    pub(crate) lists: NoHashMap<u64, Arc<dyn UpdateList>>,
}

impl ValuesList {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            values_take: NoHashMap::default(),
            static_values: NoHashMap::default(),
            datas: NoHashMap::default(),
            images: NoHashMap::default(),
            maps: NoHashMap::default(),
            lists: NoHashMap::default(),
        }
    }

    fn shrink(&mut self) {
        self.values.shrink_to_fit();
        self.values_take.shrink_to_fit();
        self.static_values.shrink_to_fit();
        self.datas.shrink_to_fit();
        self.images.shrink_to_fit();
        self.maps.shrink_to_fit();
        self.lists.shrink_to_fit();
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
        self.val.values_take.extend(creator.val.values_take);
        self.val.static_values.extend(creator.val.static_values);
        self.val.datas.extend(creator.val.datas);
        self.val.images.extend(creator.val.images);
        self.val.maps.extend(creator.val.maps);
        self.val.lists.extend(creator.val.lists);

        substate
    }

    fn value<T, Q>(&mut self, name: &str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Transportable + Send + Sync + Clone + 'static,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        let value = Value::new(name, id, type_id, value, self.sender.clone());

        self.val.values.insert(id, Arc::new(value.clone()));
        value
    }

    fn value_take<T>(&mut self, name: &str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Transportable + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        let value = ValueTake::new(name, id, type_id, self.sender.clone());

        self.val.values_take.insert(id, Arc::new(value.clone()));
        value
    }

    fn atomic<T, Q>(&mut self, name: &str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Transportable
            + Send
            + Sync
            + Clone
            + Atomic
            + 'static,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        let value = ValueAtomic::new(name, id, type_id, value, self.sender.clone());

        self.val.values.insert(id, Arc::new(value.clone()));
        value
    }

    fn add_static<T>(&mut self, name: &str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Transportable + Clone + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        let value = Static::new(name, id, type_id, value);

        self.val.static_values.insert(id, Arc::new(value.clone()));
        value
    }

    fn static_atomic<T>(&mut self, name: &'static str, value: T) -> StaticAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Transportable
            + Clone
            + Send
            + Sync
            + AtomicStatic
            + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        let value = StaticAtomic::new(name, id, type_id, value);

        self.val.static_values.insert(id, Arc::new(value.clone()));
        value
    }

    fn image(&mut self, name: &str) -> ValueImage {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueImage::new(name, id, self.sender.clone());

        self.val.images.insert(id, value.clone());
        value
    }

    fn signal<T, Q>(&mut self, name: &str) -> Signal<T, Q>
    where
        T: Serialize + Transportable + Clone + Send + Sync + 'static,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        let signal = Signal::new(id, type_id, self.sender.clone());

        signal
    }

    fn map<K, V>(&mut self, name: &str) -> ValueMap<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + Transportable + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + Transportable + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = K::get_type().get_hash() ^ V::get_type().get_hash();
        let value = ValueMap::new(name, type_id);

        self.val.maps.insert(id, Arc::new(value.clone()));
        value
    }

    fn vec<T>(&mut self, name: &str) -> ValueVec<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + Transportable + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        let value = ValueVec::new(name, type_id);

        self.val.lists.insert(id, Arc::new(value.clone()));
        value
    }

    fn data<T>(&mut self, name: &str) -> Data<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let data = Data::new(name, id, self.sender.clone());

        self.val.datas.insert(id, Arc::new(data.clone()));
        data
    }
}
