use std::collections::BTreeMap;
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
    val: BTreeMap<&'static str, Vec<ValueType>>,
    sender: MessageSender,
}

impl ParseValuesCreator {
    pub fn new() -> Self {
        let (sender, _) = MessageSender::new();

        Self {
            counter: 9, // first 10 values are reserved for special values
            val: BTreeMap::new(),
            sender,
        }
    }

    pub fn get_map(self) -> BTreeMap<&'static str, Vec<ValueType>> {
        self.val
    }

    fn get_id(&mut self) -> u32 {
        if self.counter > 16777215 {
            panic!("id counter overflow, id is 24bit long");
        }
        self.counter += 1;
        self.counter
    }

    fn add_value_type(&mut self, state: &'static str, value_type: ValueType) {
        self.val.entry(state).or_default().push(value_type);
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
        let value = ValueImage::new(id);

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
        S::new(self)
    }
}
