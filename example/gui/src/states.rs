use egui_states::Transportable;
use egui_states::{
    Data, DataMulti, Queue, Signal, State, StatesCreator, Static, StaticAtomic, Value, ValueAtomic,
    ValueImage, ValueMap, ValueTake, ValueVec,
};

#[derive(
    Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, Transportable,
)]
pub(crate) enum TestEnum {
    #[default]
    A,
    B,
    C,
}

#[derive(
    Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, Transportable,
)]
pub(crate) enum TestEnum2 {
    X,
    #[default]
    Y,
    Z,
}

#[derive(Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, Transportable)]
pub(crate) struct TestStruct {
    pub x: f32,
    pub y: f32,
    pub label: String,
}

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, Transportable)]
pub(crate) struct TestStruct2 {
    pub enabled: bool,
    pub level: u16,
    pub name: String,
}

#[derive(State)]
pub(crate) struct NestedValueStates {
    pub secondary_choice: Value<TestEnum2>,
    pub selected_enum: Value<Option<TestEnum>>,
}

#[derive(State)]
pub(crate) struct ValueStates {
    pub bool_value: Value<bool>,
    pub count: Value<i32>,
    pub ratio: ValueAtomic<f64>,
    pub queued_progress: Value<f32, Queue>,
    pub title: Value<String>,
    pub optional_value: Value<Option<i32>>,
    pub fixed_numbers: Value<[u16; 3]>,
    pub test_enum: Value<TestEnum>,
    pub nested: NestedValueStates,
}

#[derive(State)]
pub(crate) struct StaticStates {
    pub status_text: Static<String>,
    pub summary: Static<TestStruct2>,
    pub pair: StaticAtomic<[f32; 2]>,
    pub nested: NestedStaticStates,
}

#[derive(State)]
pub(crate) struct NestedStaticStates {
    pub label: Static<String>,
    pub enum_hint: Static<TestEnum>,
}

#[derive(State)]
pub(crate) struct SignalStates {
    pub empty_signal: Signal<(), Queue>,
    pub number_signal: Signal<f64>,
    pub enum_signal: Signal<TestEnum, Queue>,
}

pub(crate) struct ValueTakeStates {
    pub take_text: ValueTake<String>,
    pub take_empty: ValueTake<()>,
}

#[derive(State)]
pub(crate) struct CustomValueStates {
    pub point: Value<TestStruct>,
    pub optional_struct: Value<Option<TestStruct2>>,
}

#[derive(State)]
pub(crate) struct ValueVecActionStates {
    pub append_item: Signal<()>,
    pub remove_last: Signal<()>,
    pub reset_demo: Signal<()>,
}

#[derive(State)]
pub(crate) struct ValueVecStates {
    pub items: ValueVec<i32>,
    pub actions: ValueVecActionStates,
}

#[derive(State)]
pub(crate) struct ValueMapActionStates {
    pub insert_next: Signal<()>,
    pub remove_lowest: Signal<()>,
    pub reset_demo: Signal<()>,
}

#[derive(State)]
pub(crate) struct ValueMapStates {
    pub items: ValueMap<u16, u32>,
    pub actions: ValueMapActionStates,
}

pub(crate) struct NestedDataStates {
    pub buffer: Data<u16>,
}

pub(crate) struct DataStates {
    pub bytes: Data<u8>,
    pub samples: Data<f32>,
    pub nested: NestedDataStates,
}

pub(crate) struct NestedMultiDataStates {
    pub buffer: DataMulti<u16>,
}

pub(crate) struct MultiDataStates {
    pub bytes: DataMulti<u8>,
    pub samples: DataMulti<f32>,
    pub nested: NestedMultiDataStates,
}

#[derive(State)]
pub(crate) struct ImageStates {
    pub image: ValueImage,
}

pub struct States {
    pub(crate) values: ValueStates,
    pub(crate) signals: SignalStates,
    pub(crate) statics: StaticStates,
    pub(crate) value_take: ValueTakeStates,
    pub(crate) custom_values: CustomValueStates,
    pub(crate) value_vec: ValueVecStates,
    pub(crate) value_map: ValueMapStates,
    pub(crate) data: DataStates,
    pub(crate) multi_data: MultiDataStates,
    pub(crate) image: ImageStates,
}

impl State for ValueTakeStates {
    const NAME: &'static str = "ValueTakeStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            take_text: c.value_take("take_text"),
            take_empty: c.value_take("take_empty"),
        }
    }
}

impl State for NestedDataStates {
    const NAME: &'static str = "NestedDataStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            buffer: c.data("buffer"),
        }
    }
}

impl State for DataStates {
    const NAME: &'static str = "DataStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            bytes: c.data("bytes"),
            samples: c.data("samples"),
            nested: c.substate("nested"),
        }
    }
}

impl State for NestedMultiDataStates {
    const NAME: &'static str = "NestedMultiDataStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            buffer: c.data_multi("buffer"),
        }
    }
}

impl State for MultiDataStates {
    const NAME: &'static str = "MultiDataStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            bytes: c.data_multi("bytes"),
            samples: c.data_multi("samples"),
            nested: c.substate("nested"),
        }
    }
}

impl State for States {
    const NAME: &'static str = "States";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            values: c.substate("values"),
            signals: c.substate("signals"),
            statics: c.substate("statics"),
            value_take: c.substate("value_take"),
            custom_values: c.substate("custom_values"),
            value_vec: c.substate("value_vec"),
            value_map: c.substate("value_map"),
            data: c.substate("data"),
            multi_data: c.substate("multi_data"),
            image: c.substate("image"),
        }
    }
}
