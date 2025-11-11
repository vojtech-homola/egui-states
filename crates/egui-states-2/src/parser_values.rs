use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use egui_states_core::graphs::GraphElement;

use crate::dict::ValueDict;
use crate::graphs::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::parser::{GetInitValue, GetTypeInfo, ValueType};
use crate::sender::MessageSender;
use crate::values::{Signal, Value, ValueStatic};
use crate::{State, ValuesCreator};

pub struct ParseValuesCreator {
    counter: u32,
    states: Vec<(&'static str, Vec<ValueType>)>,
    opened_states: HashMap<&'static str, Vec<ValueType>>,
    sender: MessageSender,
}

impl ParseValuesCreator {
    pub fn new<S: State>() -> Self {
        let (sender, _) = MessageSender::new();
        let mut opened_states = HashMap::new();
        opened_states.insert(S::N, Vec::new());

        Self {
            counter: 9, // first 10 values are reserved for special values
            states: Vec::new(),
            opened_states,
            sender,
        }
    }

    pub fn get_map(mut self) -> Vec<(&'static str, Vec<ValueType>)> {
        if self.opened_states.len() > 1 {
            panic!("Not all substates were closed");
        }
        let main_state = self
            .opened_states
            .into_iter()
            .next()
            .expect("No main state opened");
        self.states.push(main_state);
        self.states
    }

    fn get_id(&mut self) -> u32 {
        self.counter += 1;
        self.counter
    }

    fn add_value_type(&mut self, state: &'static str, value_type: ValueType) {
        self.opened_states
            .get_mut(state)
            .expect("State not opened")
            .push(value_type);
    }
}

impl ValuesCreator for ParseValuesCreator {
    fn add_value<T>(&mut self, state: &'static str, name: &'static str, value: T) -> Arc<Value<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + GetInitValue + GetTypeInfo,
    {
        let id = self.get_id();
        let init = value.init_value();
        let value = Value::new(id, value, self.sender.clone());

        self.add_value_type(state, ValueType::Value(name, id, T::type_info(), init));

        value
    }

    fn add_static<T>(
        &mut self,
        state: &'static str,
        name: &'static str,
        value: T,
    ) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + GetInitValue + GetTypeInfo,
    {
        let id = self.get_id();
        let init = value.init_value();
        let value = ValueStatic::new(id, value);

        self.add_value_type(state, ValueType::Static(name, id, T::type_info(), init));

        value
    }

    fn add_image(&mut self, state: &'static str, name: &'static str) -> Arc<ValueImage> {
        let id = self.get_id();
        let value = ValueImage::new(id, self.sender.clone());

        self.add_value_type(state, ValueType::Image(name, id));

        value
    }

    fn add_signal<T>(&mut self, state: &'static str, name: &'static str) -> Arc<Signal<T>>
    where
        T: Serialize + Clone + GetTypeInfo,
    {
        let id = self.get_id();
        let signal = Signal::new(id, self.sender.clone());

        self.add_value_type(state, ValueType::Signal(name, id, T::type_info()));

        signal
    }

    fn add_dict<K, V>(&mut self, state: &'static str, name: &'static str) -> Arc<ValueDict<K, V>>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + GetTypeInfo,
        V: Clone + for<'a> Deserialize<'a> + GetTypeInfo,
    {
        let id = self.get_id();
        let value = ValueDict::new(id);

        self.add_value_type(
            state,
            ValueType::Dict(name, id, K::type_info(), V::type_info()),
        );
        value
    }

    fn add_list<T>(&mut self, state: &'static str, name: &'static str) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + GetTypeInfo,
    {
        let id = self.get_id();
        let value = ValueList::new(id);

        self.add_value_type(state, ValueType::List(name, id, T::type_info()));

        value
    }

    fn add_graphs<T>(&mut self, state: &'static str, name: &'static str) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GraphElement + GetTypeInfo,
    {
        let id = self.get_id();
        let value = ValueGraphs::new(id);

        self.add_value_type(state, ValueType::Graphs(name, id, T::type_info()));

        value
    }

    fn add_substate<S: State>(&mut self, state: &'static str, name: &'static str) -> S {
        self.add_value_type(state, ValueType::SubState(name, S::N));
        if self.opened_states.contains_key(S::N) {
            panic!("Substate {} already opened", S::N);
        }
        self.opened_states.insert(S::N, Vec::new());
        let substate = S::new(self);
        let actual = self.opened_states.remove(S::N).unwrap();
        self.states.push((S::N, actual));
        substate
    }
}
