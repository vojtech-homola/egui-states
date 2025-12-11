use std::collections::BTreeMap;
use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use egui_states_core::generate_value_id;
use egui_states_core::graphs::GraphElement;
use egui_states_core::types::GetType;

use crate::State;
use crate::build_script::values_info::{GetInitValue, StateType};
use crate::graphs::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::map::ValueMap;
use crate::sender::MessageSender;
use crate::states_creator::{StatesBuilder, StatesCreator};
use crate::values::{Signal, Value, ValueStatic};

pub struct StatesCreatorBuild {
    states: BTreeMap<&'static str, Vec<StateType>>,
    root_state: Option<&'static str>,
    sender: MessageSender,
}

impl StatesCreatorBuild {
    pub fn new() -> Self {
        let (sender, _) = MessageSender::new();

        Self {
            states: BTreeMap::new(),
            root_state: None,
            sender,
        }
    }

    pub fn get_states(self) -> BTreeMap<&'static str, Vec<StateType>> {
        self.states
    }

    pub fn root_state(&self) -> &'static str {
        self.root_state.unwrap()
    }
}

impl StatesCreator for StatesCreatorBuild {
    type Builder = StatesBuilderBuild;

    fn builder(&mut self, state_name: &'static str, parent: &String) -> StatesBuilderBuild {
        if let None = self.root_state {
            self.root_state = Some(state_name);
        }



        StatesBuilderBuild {
            state_name,
            parent: parent.clone(),
            sender: self.sender.clone(),
            states: Vec::new(),
        }
    }

    fn add_states(&mut self, builder: StatesBuilderBuild) {
        self.states.insert(builder.state_name, builder.states);
    }

    fn add_substate<S: State>(&mut self, parent: &str, name: &str) -> S {
        S::new(self, format!("{}.{}", parent, name))
    }
}

pub struct StatesBuilderBuild {
    state_name: &'static str,
    parent: String,
    sender: MessageSender,
    states: Vec<StateType>,
}

impl StatesBuilder for StatesBuilderBuild {
    fn add_value<T>(&mut self, name: &'static str, value: T) -> Arc<Value<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + GetInitValue + GetType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let init = value.init_value();
        let value = Value::new(id, value, self.sender.clone());

        self.states
            .push(StateType::Value(name, T::get_type(), init));

        value
    }

    fn add_static<T>(&mut self, name: &'static str, value: T) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + GetInitValue + GetType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let init = value.init_value();
        let value = ValueStatic::new(id, value);

        self.states
            .push(StateType::Static(name, T::get_type(), init));
        value
    }

    fn add_image(&mut self, name: &'static str) -> Arc<ValueImage> {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueImage::new(id, self.sender.clone());

        self.states.push(StateType::Image(name));

        value
    }

    fn add_signal<T>(&mut self, name: &'static str) -> Arc<Signal<T>>
    where
        T: Serialize + Clone + GetType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let signal = Signal::new(id, self.sender.clone());

        self.states.push(StateType::Signal(name, T::get_type()));

        signal
    }

    fn add_map<K, V>(&mut self, name: &'static str) -> Arc<ValueMap<K, V>>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + GetType,
        V: Clone + for<'a> Deserialize<'a> + GetType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueMap::new(id);

        self.states
            .push(StateType::Dict(name, K::get_type(), V::get_type()));
        value
    }

    fn add_list<T>(&mut self, name: &'static str) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + GetType,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueList::new(id);

        self.states.push(StateType::List(name, T::get_type()));

        value
    }

    fn add_graphs<T>(&mut self, name: &'static str) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GraphElement,
    {
        let name = format!("{}.{}", self.parent, name);
        let id = generate_value_id(&name);
        let value = ValueGraphs::new(id);

        self.states.push(StateType::Graphs(name, T::graph_type()));

        value
    }
}
