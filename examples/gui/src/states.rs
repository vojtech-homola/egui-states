use std::sync::Arc;

use egui_states::{State, Value, ValueGraphs, ValueImage, ValuesCreator};

pub struct States {
    pub(crate) value: Arc<Value<f32>>,
    pub(crate) image: Arc<ValueImage>,
    pub(crate) graphs: Arc<ValueGraphs<f32>>,
}

impl State for States {
    const N: &'static str = "States";

    fn new(c: &mut impl ValuesCreator) -> Self {
        Self {
            value: c.add_value(Self::N, "value", 0.0),
            image: c.add_image(Self::N, "image"),
            graphs: c.add_graphs(Self::N, "graphs"),
        }
    }
}
