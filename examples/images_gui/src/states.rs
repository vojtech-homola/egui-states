use std::sync::Arc;

use egui_pysync::{ValueImage, ValuesCreator, ValueStatic};

pub(crate) struct States {
    pub(crate) image: Arc<ValueImage>,
    pub(crate) text: Arc<ValueStatic<String>>,
}

impl States {
    pub(crate) fn new(c: &mut ValuesCreator) -> Self {
        Self {
            image: c.add_image(),
            text: c.add_static("unknow".to_string())
        }
    }
}
