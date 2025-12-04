use std::hash::Hash;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use egui_states_core::graphs::GraphElement;
use egui_states_core::types::GetType;

use crate::State;
use crate::build_script::values_info::GetInitValue;
use crate::graphs::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::map::ValueMap;
use crate::values::{Signal, Value, ValueStatic};

pub trait StatesCreator {
    type Builder: StatesBuilder;

    fn builder(&self, state_name: &'static str, parent: String) -> Self::Builder;
    fn add_states(&mut self, builder: Self::Builder);
    fn add_substate<S: State>(&mut self, parent: &str, name: &str) -> S;
}

pub trait StatesBuilder {
    fn add_value<T>(&mut self, name: &'static str, value: T) -> Arc<Value<T>>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetType
            + Send
            + Sync
            + Clone
            + GetInitValue
            + 'static;

    fn add_static<T>(&mut self, name: &'static str, value: T) -> Arc<ValueStatic<T>>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetType
            + Clone
            + Send
            + Sync
            + GetInitValue
            + 'static;

    fn add_signal<T>(&mut self, name: &'static str) -> Arc<Signal<T>>
    where
        T: Serialize + GetType + Clone + Send + Sync + 'static;

    fn add_image(&mut self, name: &'static str) -> Arc<ValueImage>;

    fn add_map<K, V>(&mut self, name: &'static str) -> Arc<ValueMap<K, V>>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static;

    fn add_list<T>(&mut self, name: &'static str) -> Arc<ValueList<T>>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static;

    fn add_graphs<T>(&mut self, name: &'static str) -> Arc<ValueGraphs<T>>
    where
        T: for<'a> Deserialize<'a> + GraphElement + 'static;
}
