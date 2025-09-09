use std::sync::Arc;

use egui_states::{State, Value, ValueGraphs, ValueImage, ValuesCreator};

pub(crate) struct States {
    pub(crate) value: Arc<Value<f32>>,
    pub(crate) image: Arc<ValueImage>,
    pub(crate) graphs: Arc<ValueGraphs<f32>>,
}

impl State for States {
    fn new(c: &mut ValuesCreator) -> Self {
        Self {
            value: c.add_value(0.0),
            image: c.add_image(),
            graphs: c.add_graphs(),
        }
    }
}
