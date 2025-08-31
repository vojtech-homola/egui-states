use postcard::{
    ser_flavors::{Flavor, StdVec},
    serialize_with_flavor,
};
use serde::{Deserialize, Serialize};

use crate::serialization::{HEAPLESS_SIZE, MessageData, TYPE_COMMAND};

#[derive(Serialize, Deserialize)]
pub enum ControlMessage {
    Error(String),
    Ack(u32),
    Handshake(u64, u64),
    Update(f32),
}

impl ControlMessage {
    pub fn as_str(&self) -> &str {
        match self {
            ControlMessage::Error(_) => "ErrorCommand",
            ControlMessage::Ack(_) => "AckCommand",
            ControlMessage::Handshake(_, _) => "HandshakeCommand",
            ControlMessage::Update(_) => "UpdateCommand",
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut data = StdVec::new();
        data.try_push(TYPE_COMMAND).unwrap();
        serialize_with_flavor(&self, data).unwrap()
    }

    pub fn to_data(&self) -> MessageData {
        let mut stack_data: [u8; HEAPLESS_SIZE] = [0; HEAPLESS_SIZE];
        stack_data[0] = TYPE_COMMAND;

        let len = match postcard::to_slice(self, stack_data[1..].as_mut()) {
            Ok(d) => Some(d.len() + 1),
            Err(postcard::Error::SerializeBufferFull) => None,
            Err(e) => panic!("Serialize error: {}", e),
        };

        match len {
            Some(l) => MessageData::Stack(stack_data, l),
            None => {
                let mut data = StdVec::new();
                unsafe { data.try_extend(&stack_data[0..5]).unwrap_unchecked() };
                let data =
                    serialize_with_flavor::<ControlMessage, StdVec, Vec<u8>>(self, data).unwrap();
                MessageData::Heap(data)
            }
        }
    }
}
