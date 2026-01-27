use std::hash::Hash;

use serde::{Deserialize, Serialize};

use egui_states_core::graphs::GraphElement;
use egui_states_core::types::GetType;

use crate::State;
use crate::build_script::values_info::GetInitValue;
use crate::graphs::ValueGraphs;
use crate::image::ValueImage;
use crate::list::ValueList;
use crate::map::ValueMap;
use crate::values::{GetQueueType, Signal, Static, StaticAtomic, Value, ValueAtomic};
use crate::values_atomic::Atomic;

pub trait StatesCreator {
    fn substate<S: State>(&mut self, name: &str) -> S;

    fn value<T, Q>(&mut self, name: &'static str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetType
            + Send
            + Sync
            + Clone
            + GetInitValue
            + 'static,
        Q: GetQueueType;

    fn atomic<T, Q>(&mut self, name: &'static str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetType
            + Send
            + Sync
            + Clone
            + GetInitValue
            + Atomic
            + 'static,
        Q: GetQueueType;

    fn add_static<T>(&mut self, name: &'static str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetType
            + Clone
            + Send
            + Sync
            + GetInitValue
            + 'static;

    fn static_atomic<T>(&mut self, name: &'static str, value: T) -> StaticAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + GetType
            + Clone
            + Send
            + Sync
            + GetInitValue
            + Atomic
            + 'static;

    fn signal<T, Q>(&mut self, name: &'static str) -> Signal<T, Q>
    where
        T: Serialize + GetType + Clone + Send + Sync + 'static,
        Q: GetQueueType;

    fn image(&mut self, name: &'static str) -> ValueImage;

    fn map<K, V>(&mut self, name: &'static str) -> ValueMap<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static;

    fn list<T>(&mut self, name: &'static str) -> ValueList<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + GetType + 'static;

    fn graphs<T>(&mut self, name: &'static str) -> ValueGraphs<T>
    where
        T: for<'a> Deserialize<'a> + GraphElement + 'static;
}
