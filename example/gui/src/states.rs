use std::sync::Arc;

use egui_states::{
    Signal, State, StatesCreator, Value, ValueGraphs, ValueImage, ValueList, ValueMap, ValueStatic,
};
use egui_states::{state_enum, state_struct};

#[state_enum]
pub(crate) enum TestEnum {
    A,
    B,
    C,
}

#[state_enum]
pub(crate) enum TestEnum2 {
    X,
    Y,
    Z,
}

#[state_struct]
pub(crate) struct TestStruct {
    x: f32,
    y: f32,
    label: String,
}

#[state_struct]
pub(crate) struct TestStruct2 {
    x: f32,
    y: f32,
}

pub(crate) struct MySubState {
    pub sub_value: Arc<Value<Option<i32>>>,
    pub test_enum: Arc<Value<TestEnum>>,
    pub stat: Arc<ValueStatic<[f64; 3]>>,
    pub test_signal: Arc<Signal<f64>>,
    pub signal_emp: Arc<Signal<()>>,
}

impl State for MySubState {
    const NAME: &'static str = "MySubState";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            sub_value: c.add_value("sub_value", None),
            test_enum: c.add_value("test_enum", TestEnum::A),
            stat: c.add_static("stat", [0.0, 0.0, 0.0]),
            test_signal: c.add_signal("test_signal"),
            signal_emp: c.add_signal("signal_emp"),
        }
    }
}

pub(crate) struct Collections {
    pub list: Arc<ValueList<i32>>,
    pub map: Arc<ValueMap<u16, u32>>,
}

impl State for Collections {
    const NAME: &'static str = "Collections";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            list: c.add_list("list"),
            map: c.add_map("map"),
        }
    }
}

pub struct States {
    pub(crate) value: Arc<Value<f64>>,
    pub(crate) image: Arc<ValueImage>,
    pub(crate) graphs: Arc<ValueGraphs<f32>>,
    pub(crate) test_enum: Arc<Value<TestEnum>>,
    pub(crate) test_struct: Arc<Value<TestStruct>>,
    pub(crate) test_enum2: Arc<Value<TestEnum2>>,
    pub(crate) test_struct2: Arc<Value<TestStruct2>>,
    pub(crate) my_sub_state: MySubState,
    pub(crate) map: Arc<Value<Vec<u32>>>,
    pub(crate) collections: Collections,
}

impl State for States {
    const NAME: &'static str = "States";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            value: c.add_value("value", 0.0),
            image: c.add_image("image"),
            graphs: c.add_graphs("graphs"),
            test_enum: c.add_value("test_enum", TestEnum::B),
            test_struct: c.add_value(
                "test_struct",
                TestStruct {
                    x: 5.0,
                    y: 78.0,
                    label: "tttt".to_string(),
                },
            ),
            test_enum2: c.add_value("test_enum2", TestEnum2::X),
            test_struct2: c.add_value("test_struct2", TestStruct2 { x: 0.0, y: 0.0 }),
            map: c.add_value("map", vec![78, 78, 78, 78]),

            my_sub_state: c.add_substate("my_sub_state"),
            collections: c.add_substate("collections"),
        }
    }
}
