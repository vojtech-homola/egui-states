use std::sync::Arc;

use egui_states::state_enum;
use egui_states::{State, StatesCreator, Value, ValueGraphs, ValueImage};

#[state_enum]
enum TestEnum {
    A,
    B,
    C,
}

pub struct States {
    pub(crate) value: Arc<Value<f32>>,
    pub(crate) image: Arc<ValueImage>,
    pub(crate) graphs: Arc<ValueGraphs<f32>>,
    pub(crate) test_enum: Arc<Value<TestEnum>>,
}

impl State for States {
    fn new(c: &mut StatesCreator, parent: String) -> Self {
        let mut b = c.builder("States", parent);

        let obj = Self {
            value: b.add_value("value", 0.0),
            image: b.add_image("image"),
            graphs: b.add_graphs("graphs"),
            test_enum: b.add_value("test_enum", TestEnum::A),
        };
        c.add_states(b);
        obj
    }
}
