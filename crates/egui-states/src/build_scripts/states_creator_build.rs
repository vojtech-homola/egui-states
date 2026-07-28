use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::State;
use crate::client::atomics::{Atomic, AtomicStatic};
use crate::client::data::{Data, DataMulti, private::GetDataType};
use crate::client::data_take::{DataMultiTake, DataTake};
use crate::client::image::Image;
use crate::client::initial_value::{InitValue, InitialValue};

use crate::client::messages::MessageSender;
use crate::client::states_creator::{self, StatesCreator, hash_id, hash_id_type};
use crate::client::value_map::MapState;
use crate::client::value_vec::VecState;
use crate::client::values::{
    GetQueueType, Signal, Static, StaticAtomic, Value, ValueAtomic, ValueTake,
};
use crate::data_transport::DataType;
use crate::hashing::{StableHasher, generate_value_id};
use crate::typed::{ObjectType, Typed};

#[derive(Clone, Debug, PartialEq)]
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

impl StateType {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Value(name, ..)
            | Self::ValueTake(name, ..)
            | Self::Static(name, ..)
            | Self::Image(name)
            | Self::ValueMap(name, ..)
            | Self::ValueVec(name, ..)
            | Self::Signal(name, ..)
            | Self::Data(name, ..)
            | Self::DataTake(name, ..)
            | Self::DataMulti(name, ..)
            | Self::DataMultiTake(name, ..)
            | Self::SubState(name, ..) => name,
        }
    }
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
            .push(StateType::SubState(name.to_owned(), S::NAME, states));

        substate
    }

    fn value<T, Q>(&mut self, name: &str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Typed + InitialValue,
        Q: GetQueueType,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VALUE_HASH_ID,
        );

        let init = value.init_value();
        let value = Value::new(full_name, id, type_id, value, self.sender.clone());

        self.states.push(StateType::Value(
            name.to_owned(),
            T::get_type(),
            init,
            Q::is_queue(),
        ));

        value
    }

    fn value_take<T>(&mut self, name: &str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Typed + Send + Sync + 'static,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VALUE_TAKE_HASH_ID,
        );

        let value = ValueTake::new(full_name, id, type_id, self.sender.clone());

        self.states
            .push(StateType::ValueTake(name.to_owned(), T::get_type()));

        value
    }

    fn atomic<T, Q>(&mut self, name: &str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Typed + InitialValue + Atomic,
        Q: GetQueueType,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::ATOMIC_HASH_ID,
        );

        let init = value.init_value();
        let value = ValueAtomic::new(full_name, id, type_id, value, self.sender.clone());

        self.states.push(StateType::Value(
            name.to_owned(),
            T::get_type(),
            init,
            Q::is_queue(),
        ));

        value
    }

    fn add_static<T>(&mut self, name: &str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Typed + InitialValue,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::STATIC_HASH_ID,
        );

        let init = value.init_value();
        let value = Static::new(full_name, id, type_id, value);

        self.states
            .push(StateType::Static(name.to_owned(), T::get_type(), init));
        value
    }

    fn static_atomic<T>(&mut self, name: &str, value: T) -> StaticAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Clone
            + Send
            + Sync
            + AtomicStatic
            + 'static,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::STATIC_ATOMIC_HASH_ID,
        );

        let init = value.init_value();
        let value = StaticAtomic::new(full_name, id, type_id, value);

        self.states
            .push(StateType::Static(name.to_owned(), T::get_type(), init));
        value
    }

    fn image(&mut self, name: &str) -> Image {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        hash_id(&mut self.version_hasher, id);

        let value = Image::new(full_name, id, self.sender.clone());

        self.states.push(StateType::Image(name.to_owned()));

        value
    }

    fn signal<T, Q>(&mut self, name: &str) -> Signal<T, Q>
    where
        T: Serialize + Clone + Typed,
        Q: GetQueueType,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::SIGNAL_HASH_ID,
        );

        let signal = Signal::new(id, type_id, self.sender.clone());

        self.states.push(StateType::Signal(
            name.to_owned(),
            T::get_type(),
            Q::is_queue(),
        ));

        signal
    }

    fn map<K, V>(&mut self, name: &str) -> MapState<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Typed,
        V: Clone + for<'a> Deserialize<'a> + Typed,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = V::get_type().get_hash_from(K::get_type().get_hash());
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::MAP_HASH_ID,
        );

        let value = MapState::new(full_name, type_id);

        self.states.push(StateType::ValueMap(
            name.to_owned(),
            K::get_type(),
            V::get_type(),
        ));
        value
    }

    fn vec<T>(&mut self, name: &str) -> VecState<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Typed,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VEC_HASH_ID,
        );

        let value = VecState::new(full_name, type_id);

        self.states
            .push(StateType::ValueVec(name.to_owned(), T::get_type()));

        value
    }

    fn data<T>(&mut self, name: &str) -> Data<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_HASH_ID,
        );

        let value = Data::new(full_name, id, self.sender.clone());

        self.states
            .push(StateType::Data(name.to_owned(), T::get_type()));
        value
    }

    fn data_multi<T>(&mut self, name: &str) -> DataMulti<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_MULTI_HASH_ID,
        );

        let value = DataMulti::new(full_name, id, self.sender.clone());

        self.states
            .push(StateType::DataMulti(name.to_owned(), T::get_type()));
        value
    }

    fn data_take<T>(&mut self, name: &str) -> DataTake<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_TAKE_HASH_ID,
        );

        let value = DataTake::new(full_name, id, self.sender.clone());

        self.states
            .push(StateType::DataTake(name.to_owned(), T::get_type()));
        value
    }

    fn data_multi_take<T>(&mut self, name: &str) -> DataMultiTake<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let full_name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&full_name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            states_creator::DATA_MULTI_TAKE_HASH_ID,
        );

        let value = DataMultiTake::new(full_name, id, self.sender.clone());

        self.states
            .push(StateType::DataMultiTake(name.to_owned(), T::get_type()));
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ChildState;

    impl State for ChildState {
        const NAME: &'static str = "ChildState";

        fn new(c: &mut impl StatesCreator) -> Self {
            let _: Value<i32> = c.value("count", 0);
            Self
        }
    }

    fn build(parent: &str) -> (Vec<StateType>, u64) {
        let mut builder = StatesCreatorBuild::new(parent);
        let _: ChildState = builder.substate("group.child");
        let version_hash = builder.get_version_hash();
        (builder.get_states(), version_hash)
    }

    #[test]
    fn definitions_store_local_names_while_hashes_use_full_paths() {
        let (left_states, left_hash) = build("root.left");
        let (right_states, right_hash) = build("root.right");

        assert_eq!(left_states, right_states);
        assert_ne!(left_hash, right_hash);

        let StateType::SubState(name, _, children) = &left_states[0] else {
            panic!("Expected substate");
        };
        assert_eq!(name, "child");
        assert_eq!(children[0].name(), "count");
    }
}
