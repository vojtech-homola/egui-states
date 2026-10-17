//! Value shapes that only the wire scenarios exchange between peers.
use std::collections::HashMap;

use egui_states::{Queue, Signal, Static, Value};

use crate::sections::{TestEnum, TestEnum2, TestStruct2};

#[derive(egui_states::State)]
pub struct WireTypeStates {
    pub small_unsigned: Value<u8>,
    pub small_signed: Value<i8>,
    pub short: Value<i16>,
    pub wide: Value<u64>,
    pub single: Value<f32>,
    pub tuple: Value<(i32, String, bool)>,
    pub list: Value<Vec<TestStruct2>>,
    pub table: Value<HashMap<String, f64>>,
    pub keyed_enum: Value<HashMap<TestEnum, u32>>,
    pub optional_enum: Value<Option<TestEnum2>>,
    pub bytes: Static<Vec<u8>>,
    pub pair_signal: Signal<(f32, TestEnum), Queue>,
}
