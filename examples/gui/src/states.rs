use std::sync::Arc;

use egui_states::{ValueImage, ValuesCreator, Value, State};

pub(crate) struct States {
    pub(crate) value: Arc<Value<f32>>,
    // pub(crate) image: Arc<ValueImage>,
}

impl State for States {
    fn new(c: &mut ValuesCreator) -> Self {
        Self {
            value: c.add_value(0.0),
            // image: c.add_image(),
        }
    }
}
