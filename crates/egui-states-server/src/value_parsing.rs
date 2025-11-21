use bytes::Bytes;
use serde::{Deserialize, Serialize};

use egui_states_core::serialization::{
    SerResult, deserialize_value, serialize_value_slice, serialize_value_vec,
};

pub(crate) struct ValueParser {
    value: Bytes,
    pointer: usize,
}

impl ValueParser {
    pub(crate) fn new(value: Bytes) -> Self {
        Self { value, pointer: 0 }
    }

    pub(crate) fn get<T: for<'a> Deserialize<'a>>(&mut self, value: &mut T) -> bool {
        let result = deserialize_value(&self.value[self.pointer..]);
        match result {
            Some((val, size)) => {
                *value = val;
                self.pointer += size;
                true
            }
            None => false,
        }
    }
}

pub(crate) enum SerData {
    Stack([u8; 32], usize),
    Heap(Vec<u8>),
}

pub(crate) struct ValueCreator {
    data: SerData,
}

impl ValueCreator {
    pub(crate) fn new() -> Self {
        Self {
            data: SerData::Stack([0u8; 32], 0),
        }
    }

    pub(crate) fn add<T: Serialize>(&mut self, value: &T) -> bool {
        match &mut self.data {
            SerData::Stack(d, size) => match serialize_value_slice(value, d[*size..].as_mut()) {
                SerResult::Ok(s) => {
                    *size += s;
                    true
                }
                SerResult::Heap(vec) => {
                    let mut new_vec = Vec::with_capacity(*size + vec.len());
                    new_vec.extend_from_slice(&d[..*size]);
                    new_vec.extend_from_slice(&vec);
                    self.data = SerData::Heap(new_vec);
                    true
                }
            },
            SerData::Heap(data) => serialize_value_vec(value, data),
        }
    }

    pub(crate) fn finalize(self) -> Bytes {
        match self.data {
            SerData::Stack(d, size) => Bytes::copy_from_slice(&d[..size]),
            SerData::Heap(vec) => Bytes::from_owner(vec),
        }
    }
}
