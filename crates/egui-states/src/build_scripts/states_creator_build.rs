use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::State;
use crate::client::atomics::{Atomic, AtomicStatic};
use crate::client::data::{Data, DataMulti, private::GetDataType};
use crate::client::data_take::{DataMultiTake, DataTake};
use crate::client::image::Image;

use crate::client::messages::MessageSender;
use crate::client::states_creator::{self, StatesCreator, hash_id, hash_id_type};
use crate::client::value_map::MapState;
use crate::client::value_vec::VecState;
use crate::client::values::{
    GetQueueType, Signal, Static, StaticAtomic, Value, ValueAtomic, ValueTake,
};
use crate::data_transport::DataType;
use crate::hashing::{StableHasher, generate_value_id};
use crate::transport::{InitValue, ObjectType, Transportable};

#[derive(Clone)]
pub(crate) enum StateType {
    Value(String, ObjectType, InitValue, bool),
    ValueTake(String, ObjectType),
    Static(String, ObjectType, InitValue),
    Image(String),
    ValueMap(String, ObjectType, ObjectType),
    ValueVec(String, ObjectType),
    Signal(String, ObjectType, bool),
    Data(String, DataType),
    DataTake(String, DataType),
    DataMulti(String, DataType),
    DataMultiTake(String, DataType),
    SubState(String, &'static str, Vec<StateType>),
}

pub(crate) struct StatesCreatorBuild {
    states: Vec<StateType>,
    parent: String,
    sender: MessageSender,
    version_hasher: StableHasher,
}

impl StatesCreatorBuild {
    pub fn new(parent: &str) -> Self {
        let (sender, _) = MessageSender::new();

        Self {
            states: Vec::new(),
            parent: parent.to_string(),
            sender,
            version_hasher: StableHasher::new(),
        }
    }

    pub fn get_version_hash(&mut self) -> u64 {
        self.version_hasher.finish()
    }

    pub fn get_states(self) -> Vec<StateType> {
        self.states
    }
}

impl StatesCreator for StatesCreatorBuild {
    fn substate<S: State>(&mut self, name: &str) -> S {
        let parent = format!("{}.{}", self.parent, name);

        let mut builder = Self::new(&parent);
        let substate = S::new(&mut builder);
        builder
            .version_hasher
            .finish()
            .hash(&mut self.version_hasher);

        let states = builder.get_states();
        self.states
            .push(StateType::SubState(parent, S::NAME, states));

        substate
    }

    fn value<T, Q>(&mut self, name: &'static str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Transportable,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VALUE_HASH_ID,
        );

        let init = value.init_value();
        let value = Value::new(name.clone(), id, type_id, value, self.sender.clone());

        self.states
            .push(StateType::Value(name, T::get_type(), init, Q::is_queue()));

        value
    }

    fn value_take<T>(&mut self, name: &'static str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Transportable + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VALUE_TAKE_HASH_ID,
        );

        let value = ValueTake::new(name.clone(), id, type_id, self.sender.clone());

        self.states
            .push(StateType::ValueTake(name.clone(), T::get_type()));

        value
    }

    fn atomic<T, Q>(&mut self, name: &'static str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Transportable + Atomic,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::ATOMIC_HASH_ID,
        );

        let init = value.init_value();
        let value = ValueAtomic::new(name.clone(), id, type_id, value, self.sender.clone());

        self.states
            .push(StateType::Value(name, T::get_type(), init, Q::is_queue()));

        value
    }

    fn add_static<T>(&mut self, name: &'static str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Transportable,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::STATIC_HASH_ID,
        );

        let init = value.init_value();
        let value = Static::new(name.clone(), id, type_id, value);

        self.states
            .push(StateType::Static(name, T::get_type(), init));
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
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::STATIC_ATOMIC_HASH_ID,
        );

        let init = value.init_value();
        let value = StaticAtomic::new(name.clone(), id, type_id, value);

        self.states
            .push(StateType::Static(name, T::get_type(), init));
        value
    }

    fn image(&mut self, name: &'static str) -> Image {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        hash_id(&mut self.version_hasher, id);

        let value = Image::new(name.clone(), id, self.sender.clone());

        self.states.push(StateType::Image(name));

        value
    }

    fn signal<T, Q>(&mut self, name: &'static str) -> Signal<T, Q>
    where
        T: Serialize + Clone + Transportable,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::SIGNAL_HASH_ID,
        );

        let signal = Signal::new(id, type_id, self.sender.clone());

        self.states
            .push(StateType::Signal(name, T::get_type(), Q::is_queue()));

        signal
    }

    fn map<K, V>(&mut self, name: &'static str) -> MapState<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Transportable,
        V: Clone + for<'a> Deserialize<'a> + Transportable,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = V::get_type().get_hash_from(K::get_type().get_hash());
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::MAP_HASH_ID,
        );

        let value = MapState::new(name.clone(), type_id);

        self.states
            .push(StateType::ValueMap(name, K::get_type(), V::get_type()));
        value
    }

    fn vec<T>(&mut self, name: &'static str) -> VecState<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Transportable,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VEC_HASH_ID,
        );

        let value = VecState::new(name.clone(), type_id);

        self.states.push(StateType::ValueVec(name, T::get_type()));

        value
    }

    fn data<T>(&mut self, name: &'static str) -> Data<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_HASH_ID,
        );

        let value = Data::new(name.clone(), id, self.sender.clone());

        self.states.push(StateType::Data(name, T::get_type()));
        value
    }

    fn data_multi<T>(&mut self, name: &'static str) -> DataMulti<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_MULTI_HASH_ID,
        );

        let value = DataMulti::new(name.clone(), id, self.sender.clone());

        self.states.push(StateType::DataMulti(name, T::get_type()));
        value
    }

    fn data_take<T>(&mut self, name: &'static str) -> DataTake<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_TAKE_HASH_ID,
        );

        let value = DataTake::new(name.clone(), id, self.sender.clone());

        self.states.push(StateType::DataTake(name, T::get_type()));
        value
    }

    fn data_multi_take<T>(&mut self, name: &'static str) -> DataMultiTake<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_MULTI_TAKE_HASH_ID,
        );

        let value = DataMultiTake::new(name.clone(), id, self.sender.clone());

        self.states
            .push(StateType::DataMultiTake(name, T::get_type()));
        value
    }
}
