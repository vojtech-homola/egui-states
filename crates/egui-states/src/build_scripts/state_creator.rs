use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::State;
use crate::client::atomics::{Atomic, AtomicStatic};
use crate::client::graphs::ValueGraphs;
use crate::client::image::ValueImage;
use crate::client::list::ValueList;
use crate::client::map::ValueMap;
use crate::client::sender::MessageSender;
use crate::client::states_creator::StatesCreator;
use crate::client::values::{GetQueueType, Signal, Static, StaticAtomic, Value, ValueAtomic};
use crate::graphs::{GraphElement, GraphType};
use crate::hashing::generate_value_id;
use crate::transport::{InitValue, ObjectType, Transportable};

#[derive(Clone)]
pub enum StateType {
    Value(String, ObjectType, InitValue, bool),
    Static(String, ObjectType, InitValue),
    Image(String),
    Map(String, ObjectType, ObjectType),
    List(String, ObjectType),
    Graphs(String, GraphType),
    Signal(String, ObjectType, bool),
    SubState(String, &'static str, Vec<StateType>),
}

pub(crate) struct StatesCreatorBuild {
    states: Vec<StateType>,
    parent: String,
    sender: MessageSender,
}

impl StatesCreatorBuild {
    pub fn new(parent: &str) -> Self {
        let (sender, _) = MessageSender::new();

        Self {
            states: Vec::new(),
            parent: parent.to_string(),
            sender,
        }
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
        let init = value.init_value();
        let value = Value::new(name.clone(), id, value, self.sender.clone());

        self.states
            .push(StateType::Value(name, T::get_type(), init, Q::is_queue()));

        value
    }

    fn atomic<T, Q>(&mut self, name: &'static str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Transportable + Atomic,
        Q: GetQueueType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let init = value.init_value();
        let value = ValueAtomic::new(name.clone(), id, value, self.sender.clone());

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
        let init = value.init_value();
        let value = Static::new(name.clone(), id, value);

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
        let init = value.init_value();
        let value = StaticAtomic::new(name.clone(), id, value);

        self.states
            .push(StateType::Static(name, T::get_type(), init));
        value
    }

    fn image(&mut self, name: &'static str) -> ValueImage {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueImage::new(name.clone(), id, self.sender.clone());

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
        let signal = Signal::new(id, self.sender.clone());

        self.states
            .push(StateType::Signal(name, T::get_type(), Q::is_queue()));

        signal
    }

    fn map<K, V>(&mut self, name: &'static str) -> ValueMap<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Transportable,
        V: Clone + for<'a> Deserialize<'a> + Transportable,
    {
        let name = format!("{}.{}", self.parent, name);
        let value = ValueMap::new(name.clone());

        self.states
            .push(StateType::Map(name, K::get_type(), V::get_type()));
        value
    }

    fn list<T>(&mut self, name: &'static str) -> ValueList<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Transportable,
    {
        let name = format!("{}.{}", self.parent, name);
        let value = ValueList::new(name.clone());

        self.states.push(StateType::List(name, T::get_type()));

        value
    }

    fn graphs<T>(&mut self, name: &'static str) -> ValueGraphs<T>
    where
        T: for<'a> Deserialize<'a> + GraphElement,
    {
        let name = format!("{}.{}", self.parent, name);
        let value = ValueGraphs::new(name.clone());

        self.states.push(StateType::Graphs(name, T::graph_type()));

        value
    }
}
