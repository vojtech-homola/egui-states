use bytes::Bytes;
use serde::{Deserialize, Serialize};

use egui_states_core::serialization::{FastVec, deserialize_value, serialize_to_data};

pub(crate) struct ValueParser {
    value: Bytes,
    pointer: usize,
}

impl ValueParser {
    pub(crate) fn new(value: Bytes) -> Self {
        Self { value, pointer: 0 }
    }

    pub(crate) fn get<T: for<'a> Deserialize<'a>>(&mut self, value: &mut T) -> Result<(), ()> {
        deserialize_value(&self.value[self.pointer..]).map(|(val, size)| {
            *value = val;
            self.pointer += size;
        })
    }
}

pub(crate) struct ValueCreator {
    data: Option<FastVec<32>>,
}

impl ValueCreator {
    pub(crate) fn new() -> Self {
        Self {
            data: Some(FastVec::new()),
        }
    }

    pub(crate) fn add<T: Serialize>(&mut self, value: &T) {
        if let Some(data) = self.data.take() {
            self.data = Some(serialize_to_data(value, data));
        }
    }

    pub(crate) fn finalize(self) -> Bytes {
        match self.data {
            Some(data) => data.to_bytes(),
            None => Bytes::new(), // Should not happen
        }
    }
}
