use egui_states::{
    Queue, Signal, State, StatesCreator, Static, StaticAtomic, Value, ValueAtomic, ValueGraphs, ValueImage,
    ValueList, ValueMap,
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
    pub sub_value: Value<Option<i32>>,
    pub test_enum: Value<TestEnum, Queue>,
    pub stat: StaticAtomic<[f32; 2]>,
    pub test_signal: Signal<f64>,
    pub signal_emp: Signal<()>,
}

impl State for MySubState {
    const NAME: &'static str = "MySubState";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            sub_value: c.value("sub_value", None),
            test_enum: c.value("test_enum", TestEnum::A),
            stat: c.static_atomic("stat", [0.0, 0.0]),
            test_signal: c.signal("test_signal"),
            signal_emp: c.signal("signal_emp"),
        }
    }
}

pub(crate) struct Collections {
    pub list: ValueList<i32>,
    pub map: ValueMap<u16, u32>,
}

impl State for Collections {
    const NAME: &'static str = "Collections";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            list: c.list("list"),
            map: c.map("map"),
        }
    }
}

pub struct States {
    pub(crate) value: ValueAtomic<f64>,
    pub(crate) value2: Value<f32>,
    pub(crate) empty_signal: Signal<(), Queue>,
    pub(crate) image: ValueImage,
    pub(crate) graphs: ValueGraphs<f32>,
    pub(crate) test_enum: Static<TestEnum>,
    pub(crate) test_struct: Value<TestStruct>,
    pub(crate) test_enum2: Value<TestEnum2>,
    pub(crate) test_struct2: Value<TestStruct2>,
    pub(crate) my_sub_state: MySubState,
    pub(crate) map: Value<Vec<u32>>,
    pub(crate) collections: Collections,
}

impl State for States {
    const NAME: &'static str = "States";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            value: c.atomic("value", 0.0),
            value2: c.value("value2", 0.0f32),
            empty_signal: c.signal("empty_signal"),
            image: c.image("image"),
            graphs: c.graphs("graphs"),
            test_enum: c.add_static("test_enum", TestEnum::B),
            test_struct: c.value(
                "test_struct",
                TestStruct {
                    x: 5.0,
                    y: 78.0,
                    label: "tttt".to_string(),
                },
            ),
            test_enum2: c.value("test_enum2", TestEnum2::X),
            test_struct2: c.value("test_struct2", TestStruct2 { x: 0.0, y: 0.0 }),
            map: c.value("map", vec![78, 78, 78, 78]),

            my_sub_state: c.substate("my_sub_state"),
            collections: c.substate("collections"),
        }
    }
}
