use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::serialization::{FastVec, deserialize_value, serialize_to_data};

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
    data: FastVec<32>,
}

impl ValueCreator {
    pub(crate) fn new() -> Self {
        Self {
            data: FastVec::new(),
        }
    }

    #[inline]
    pub(crate) fn add<T: Serialize>(&mut self, value: &T) -> Result<(), ()> {
        serialize_to_data(value, &mut self.data)
    }

    pub(crate) fn finalize(self) -> Bytes {
        self.data.to_bytes()
    }
}
