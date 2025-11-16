use std::marker::PhantomData;

use egui_states_core::values::ObjectType;

pub(crate) struct ValueParser {
    value_type: ObjectType,
}

impl ValueParser {
    pub(crate) fn new(value_type: ObjectType) -> Self {
        Self { value_type }
    }
}

pub(crate) struct ValueCreator {
    _phantom: PhantomData<()>,
}

impl ValueCreator {
    
}