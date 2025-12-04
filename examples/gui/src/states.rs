use std::sync::Arc;

use egui_states::{State, StatesBuilder, StatesCreator, Value, ValueGraphs, ValueImage};
use egui_states::{state_enum, state_struct};

#[state_enum]
enum TestEnum {
    A,
    B,
    C,
}

#[state_struct]
struct TestStruct {
    x: f32,
    y: f32,
    label: String,
}

pub struct States {
    pub(crate) value: Arc<Value<f32>>,
    pub(crate) image: Arc<ValueImage>,
    pub(crate) graphs: Arc<ValueGraphs<f32>>,
    pub(crate) test_enum: Arc<Value<TestEnum>>,
    pub(crate) test_struct: Arc<Value<TestStruct>>,
}

impl State for States {
    fn new(c: &mut impl StatesCreator, parent: String) -> Self {
        let mut b = c.builder("States", parent);

        let obj = Self {
            value: b.add_value("value", 0.0),
            image: b.add_image("image"),
            graphs: b.add_graphs("graphs"),
            test_enum: b.add_value("test_enum", TestEnum::A),
            test_struct: b.add_value(
                "test_struct",
                TestStruct {
                    x: 0.0,
                    y: 0.0,
                    label: String::new(),
                },
            ),
        };
        c.add_states(b);
        obj
    }
}
