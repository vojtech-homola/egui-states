use egui_states::{Queue, Signal, Static, StaticAtomic, Value, ValueAtomic};

#[egui_states::typed(rust_derive(Debug, PartialEq, Eq))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, egui_states::InitialValue)]
pub enum TestEnum {
    #[default]
    A,
    B,
    C,
}

#[egui_states::typed(rust_derive(Debug, PartialEq, Eq))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, egui_states::InitialValue)]
pub enum TestEnum2 {
    X = 3,
    #[default]
    Y = 17,
    Z = 201,
}

#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Debug, Default, PartialEq, egui_states::InitialValue)]
pub struct TestStruct {
    pub x: f32,
    pub y: f32,
    pub label: String,
}

#[egui_states::typed(rust_derive(Debug, PartialEq, Eq, Hash))]
#[derive(Clone, Debug, Default, PartialEq, Eq, egui_states::InitialValue)]
pub struct TestStruct2 {
    pub enabled: bool,
    pub level: u16,
    pub name: String,
}

#[derive(egui_states::State)]
pub struct NestedValueStates {
    pub secondary_choice: Value<TestEnum2>,
    pub selected_enum: Value<Option<TestEnum>>,
}

#[derive(egui_states::State)]
pub struct ValueStates {
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

#[derive(egui_states::State)]
pub struct StaticStates {
    pub status_text: Static<String>,
    pub summary: Static<TestStruct2>,
    pub pair: StaticAtomic<[f32; 2]>,
    pub nested: NestedStaticStates,
}

#[derive(egui_states::State)]
pub struct NestedStaticStates {
    pub label: Static<String>,
    pub enum_hint: Static<TestEnum>,
}

#[derive(egui_states::State)]
pub struct SignalStates {
    pub empty_signal: Signal<(), Queue>,
    pub number_signal: Signal<f64>,
    pub enum_signal: Signal<TestEnum, Queue>,
}

#[derive(egui_states::State)]
pub struct CustomValueStates {
    pub point: Value<TestStruct>,
    pub optional_struct: Value<Option<TestStruct2>>,
}

#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Debug, Default, PartialEq, egui_states::InitialValue)]
pub struct WirePacket {
    pub signed: i64,
    pub unsigned: u64,
    pub text: String,
    pub optional: Option<i32>,
    pub fixed: [u16; 3],
    pub nested: TestStruct2,
    pub choice: TestEnum2,
}
