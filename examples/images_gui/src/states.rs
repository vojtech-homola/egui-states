use std::sync::Arc;

use egui_states::{ValueImage, ValuesCreator};

pub(crate) struct States {
    pub(crate) image: Arc<ValueImage>,
}

impl States {
    pub(crate) fn new(c: &mut ValuesCreator) -> Self {
        Self {
            image: c.add_image(),
        }
    }
}
