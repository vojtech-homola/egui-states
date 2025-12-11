use std::sync::Arc;

use egui_states::{
    Signal, State, StatesBuilder, StatesCreator, Value, ValueGraphs, ValueImage, ValueList,
    ValueMap, ValueStatic,
};
use egui_states::{state_enum, state_struct};

#[state_enum]
enum TestEnum {
    A,
    B,
    C,
}

#[state_enum]
enum TestEnum2 {
    X,
    Y,
    Z,
}

#[state_struct]
struct TestStruct {
    x: f32,
    y: f32,
    label: String,
}

#[state_struct]
struct TestStruct2 {
    x: f32,
    y: f32,
    lab: Vec<(i64, String)>,
}

pub(crate) struct MySubState {
    pub sub_value: Arc<Value<i32>>,
    pub test_enum: Arc<Value<TestEnum>>,
    pub stat: Arc<ValueStatic<[f32; 3]>>,
    pub test_signal: Arc<Signal<f32>>,
    pub signal_emp: Arc<Signal<()>>,
}

impl State for MySubState {
    fn new(c: &mut impl StatesCreator, parent: String) -> Self {
        let mut b = c.builder("MySubState", &parent);

        let obj = Self {
            sub_value: b.add_value("sub_value", 0),
            test_enum: b.add_value("test_enum", TestEnum::A),
            stat: b.add_static("stat", [0.0, 0.0, 0.0]),
            test_signal: b.add_signal("test_signal"),
            signal_emp: b.add_signal("signal_emp"),
        };

        c.add_states(b);
        obj
    }
}

pub(crate) struct Collections {
    pub list: Arc<ValueList<i32>>,
    pub map: Arc<ValueMap<u16, String>>,
}

impl State for Collections {
    fn new(c: &mut impl StatesCreator, parent: String) -> Self {
        let mut b = c.builder("Collections", &parent);

        let obj = Self {
            list: b.add_list("list"),
            map: b.add_map("map"),
        };

        c.add_states(b);
        obj
    }
}

pub struct States {
    pub(crate) value: Arc<Value<f32>>,
    pub(crate) image: Arc<ValueImage>,
    pub(crate) graphs: Arc<ValueGraphs<f32>>,
    pub(crate) test_enum: Arc<Value<TestEnum>>,
    pub(crate) test_struct: Arc<Value<TestStruct>>,
    pub(crate) test_enum2: Arc<Value<TestEnum2>>,
    pub(crate) test_struct2: Arc<Value<TestStruct2>>,
    pub(crate) my_sub_state: MySubState,
    pub(crate) collections: Collections,
}

impl State for States {
    fn new(c: &mut impl StatesCreator, parent: String) -> Self {
        let mut b = c.builder("States", &parent);

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
                    label: "".to_string(),
                },
            ),
            test_enum2: b.add_value("test_enum2", TestEnum2::X),
            test_struct2: b.add_value(
                "test_struct2",
                TestStruct2 {
                    x: 0.0,
                    y: 0.0,
                    lab: Vec::new(),
                },
            ),

            my_sub_state: c.add_substate(&parent, "my_sub_state"),
            collections: c.add_substate(&parent, "collections"),
        };

        c.add_states(b);
        obj
    }
}
