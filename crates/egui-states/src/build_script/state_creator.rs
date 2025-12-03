use std::collections::BTreeMap;
use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use egui_states_core::generate_value_id;
use egui_states_core::graphs::GraphElement;
use egui_states_core::types::GetType;

use crate::State;
use crate::values_info::{GetInitValue, GetTypeInfo, ValueType};
use crate::graphs::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::map::ValueMap;
use crate::sender::MessageSender;
use crate::values::{Signal, Value, ValueStatic};

pub struct StatesCreator {
    states: BTreeMap<&'static str, Vec<ValueType>>,
    sender: MessageSender,
}

impl StatesCreator {
    pub fn new() -> Self {
        let (sender, _) = MessageSender::new();

        Self {
            states: BTreeMap::new(),
            sender,
        }
    }

    pub fn get_states(self) -> BTreeMap<&'static str, Vec<ValueType>> {
        self.states
    }

    pub fn builder(&self, state_name: &'static str, parent: String) -> StatesBuilder {
        StatesBuilder {
            state_name,
            parent,
            sender: self.sender.clone(),
            states: Vec::new(),
        }
    }

    pub fn add_states(&mut self, builder: StatesBuilder) {
        self.states.insert(builder.state_name, builder.states);
    }

    pub fn add_substate<S: State>(&mut self, parent: &str, name: &str) -> S {
        S::new(self, format!("{}.{}", parent, name))
    }
}

pub struct StatesBuilder {
    state_name: &'static str,
    parent: String,
    sender: MessageSender,
    states: Vec<ValueType>,
}

impl StatesBuilder {
    pub fn add_value<T>(&mut self, name: &str, value: T) -> Arc<Value<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + GetInitValue + GetTypeInfo,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let init = value.init_value();
        let value = Value::new(id, value, self.sender.clone());

        self.states
            .push(ValueType::Value(name, T::type_info(), init));

        value
    }

    pub fn add_static<T>(&mut self, name: &'static str, value: T) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + GetInitValue + GetTypeInfo,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let init = value.init_value();
        let value = ValueStatic::new(id, value);

        self.states
            .push(ValueType::Static(name, T::type_info(), init));
        value
    }

    pub fn add_image(&mut self, name: &'static str) -> Arc<ValueImage> {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueImage::new(id, self.sender.clone());

        self.states.push(ValueType::Image(name));

        value
    }

    pub fn add_signal<T>(&mut self, name: &'static str) -> Arc<Signal<T>>
    where
        T: Serialize + Clone + GetTypeInfo,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let signal = Signal::new(id, self.sender.clone());

        self.states.push(ValueType::Signal(name, T::type_info()));

        signal
    }

    pub fn add_dict<K, V>(&mut self, name: &'static str) -> Arc<ValueMap<K, V>>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + GetTypeInfo + GetType,
        V: Clone + for<'a> Deserialize<'a> + GetTypeInfo + GetType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueMap::new(id);

        self.states
            .push(ValueType::Dict(name, K::type_info(), V::type_info()));
        value
    }

    pub fn add_list<T>(&mut self, name: &'static str) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + GetTypeInfo + GetType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueList::new(id);

        self.states.push(ValueType::List(name, T::type_info()));

        value
    }

    pub fn add_graphs<T>(&mut self, name: &'static str) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GetTypeInfo + GraphElement,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueGraphs::new(id);

        self.states.push(ValueType::Graphs(name, T::type_info()));

        value
    }
}
