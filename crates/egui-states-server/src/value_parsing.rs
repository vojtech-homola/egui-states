use std::marker::PhantomData;

use bytes::Bytes;

use egui_states_core::values::ObjectType;

pub(crate) struct ValueParser {
    value: Bytes,
    pointer: usize,
}

impl ValueParser {
    pub(crate) fn new(value: Bytes) -> Self {
        Self {
            value,
            pointer: 0,
        }
    }

    pub(crate) fn get_u8(&mut self, value: &mut u8) -> bool {
        let ptr = self.value.as_ref();

        false
    }
}

pub(crate) struct ValueCreator {
    _phantom: PhantomData<()>,
}

impl ValueCreator {}
