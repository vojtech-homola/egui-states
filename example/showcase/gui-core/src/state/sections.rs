use egui_states::{Signal, Static, StaticAtomic, Value, ValueAtomic};

#[egui_states::typed]
#[derive(Clone, Copy, Default, PartialEq, Eq, egui_states::InitialValue)]
pub enum PrimaryChoice {
    #[default]
    A,
    B,
    C,
}

#[egui_states::typed]
#[derive(Clone, Copy, Default, PartialEq, Eq, egui_states::InitialValue)]
pub enum SecondaryChoice {
    X,
    #[default]
    Y,
    Z,
}

#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub label: String,
}

#[egui_states::typed(rust_derive(Debug, PartialEq, Eq, Hash))]
#[derive(Clone, Default, PartialEq, Eq, egui_states::InitialValue)]
pub struct Summary {
    pub enabled: bool,
    pub level: u16,
    pub name: String,
}

#[derive(egui_states::State)]
pub struct NestedValueStates {
    pub secondary_choice: Value<SecondaryChoice>,
    pub selected_enum: Value<Option<PrimaryChoice>>,
}

#[derive(egui_states::State)]
pub struct ValueStates {
    pub bool_value: Value<bool>,
    pub count: Value<i32>,
    pub ratio: ValueAtomic<f64>,
    #[state(queue)]
    pub queued_progress: Value<f32>,
    pub title: Value<String>,
    pub optional_value: Value<Option<i32>>,
    pub fixed_numbers: Value<[u16; 3]>,
    pub primary_choice: Value<PrimaryChoice>,
    pub nested: NestedValueStates,
}

#[derive(egui_states::State)]
pub struct StaticStates {
    pub status_text: Static<String>,
    pub summary: Static<Summary>,
    pub pair: StaticAtomic<[f32; 2]>,
    pub nested: NestedStaticStates,
}

#[derive(egui_states::State)]
pub struct NestedStaticStates {
    pub label: Static<String>,
    pub enum_hint: Static<PrimaryChoice>,
}

#[derive(egui_states::State)]
pub struct SignalStates {
    #[state(queue)]
    pub empty_signal: Signal<()>,
    pub number_signal: Signal<f64>,
    #[state(queue)]
    pub enum_signal: Signal<PrimaryChoice>,
}

#[derive(egui_states::State)]
pub struct CustomValueStates {
    pub point: Value<Point>,
    pub optional_struct: Value<Option<Summary>>,
}
