use egui_states::Transportable;
use egui_states::{
    Data, Queue, Signal, State, Static, StaticAtomic, Value, ValueAtomic, ValueImage, ValueMap,
    ValueTake, ValueVec,
};

#[derive(
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    Transportable,
)]
pub(crate) enum TestEnum {
    #[default]
    A,
    B,
    C,
}

#[derive(
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    Transportable,
)]
pub(crate) enum TestEnum2 {
    X,
    #[default]
    Y,
    Z,
}

#[derive(
    Clone,
    Default,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    Transportable,
)]
pub(crate) struct TestStruct {
    pub x: f32,
    pub y: f32,
    pub label: String,
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    Transportable,
)]
pub(crate) struct TestStruct2 {
    pub enabled: bool,
    pub level: u16,
    pub name: String,
}

#[derive(State)]
pub(crate) struct ScalarStates {
    pub bool_value: Value<bool>,
    pub count: Value<i32>,
    pub ratio: ValueAtomic<f64>,
    pub queued_progress: Value<f32, Queue>,
    pub title: Value<String>,
    pub optional_value: Value<Option<i32>>,
    pub fixed_numbers: Value<[u16; 3]>,
    pub test_enum: Value<TestEnum>,
}

#[derive(State)]
pub(crate) struct StaticStates {
    pub status_text: Static<String>,
    pub summary: Static<TestStruct2>,
    pub pair: StaticAtomic<[f32; 2]>,
}

#[derive(State)]
pub(crate) struct CustomStates {
    pub point: Value<TestStruct>,
    pub choice: Value<TestEnum2>,
    pub optional_struct: Value<Option<TestStruct2>>,
}

#[derive(State)]
pub(crate) struct CollectionStates {
    pub plain_vec_value: Value<Vec<u32>>,
    pub list: ValueVec<i32>,
    pub map: ValueMap<u16, u32>,
}

#[derive(State)]
pub(crate) struct EventStates {
    pub empty_signal: Signal<(), Queue>,
    pub number_signal: Signal<f64>,
    pub enum_signal: Signal<TestEnum, Queue>,
    pub take_text: ValueTake<String>,
    pub take_empty: ValueTake<()>,
}

#[derive(State)]
pub(crate) struct DataStates {
    pub image: ValueImage,
    pub bytes: Data<u8>,
    pub samples: Data<f32>,
}

#[derive(State)]
pub(crate) struct NestedLeafStates {
    pub enabled: Value<bool>,
    pub message: Value<String>,
    pub buffer: Data<u16>,
}

#[derive(State)]
pub(crate) struct NestedInnerStates {
    pub selected: Value<Option<TestEnum>>,
    pub pair: StaticAtomic<[f32; 2]>,
    pub leaf: NestedLeafStates,
}

#[derive(State)]
pub(crate) struct NestedStates {
    pub label: Static<String>,
    pub counter: Value<i32, Queue>,
    pub inner: NestedInnerStates,
}

#[derive(State)]
pub struct States {
    pub(crate) scalars: ScalarStates,
    pub(crate) statics: StaticStates,
    pub(crate) custom: CustomStates,
    pub(crate) collections: CollectionStates,
    pub(crate) events: EventStates,
    pub(crate) data: DataStates,
    pub(crate) nested: NestedStates,
}
