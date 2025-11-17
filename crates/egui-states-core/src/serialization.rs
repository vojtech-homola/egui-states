use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::collections::{ListHeader, MapHeader};
use crate::controls::ControlMessage;
use crate::graphs::GraphHeader;
use crate::image::ImageHeader;

#[derive(Serialize, Deserialize)]
pub enum ServerHeader {
    Value(u64, u64, bool),
    Static(u64, u64, bool),
    Image(u64, bool, ImageHeader),
    Graph(u64, bool, GraphHeader),
    List(u64, u64, bool, ListHeader),
    Map(u64, (u64, u64), bool, MapHeader),
    Control(ControlMessage),
}

#[derive(Serialize, Deserialize)]
pub enum ClientHeader {
    Value(u64, u64, bool),
    Signal(u64, u64),
    Control(ControlMessage),
}

impl ClientHeader {
    pub fn ack(id: u64) -> Self {
        ClientHeader::Control(ControlMessage::Ack(id))
    }

    pub fn error(message: String) -> Self {
        ClientHeader::Control(ControlMessage::Error(message))
    }

    pub fn serialize_handshake(id: u64, version: u64) -> Bytes {
        let header = ClientHeader::Control(ControlMessage::Handshake(id, version));
        let data =
            postcard::to_vec::<ClientHeader, HEAPLESS_SIZE>(&header).expect("Failed to serialize");
        Bytes::from_owner(data)
    }

    pub fn serialize_handshake_vec(id: u64, version: u64) -> Vec<u8> {
        let header = ClientHeader::Control(ControlMessage::Handshake(id, version));
        let data = postcard::to_stdvec(&header).expect("Failed to serialize");
        data
    }

    pub fn deserialize_header(message: &Bytes) -> Result<(Self, Option<Bytes>), &'static str> {
        let (header, rest) = postcard::take_from_bytes::<ClientHeader>(&message)
            .map_err(|_| "Failed to deserialize")?;

        let l = rest.len();
        let data = match header {
            Self::Control(_) => {
                if l != 0 {
                    return Err("Control message should not have data");
                }
                None
            }
            _ => {
                if l == 0 {
                    return Err("Non-control message should have data");
                }
                Some(message.slice(message.len() - rest.len()..))
            }
        };

        Ok((header, data))
    }

    pub fn serialize_message(&self, data: Option<MessageData>) -> Bytes {
        match data {
            Some(MessageData::Heap(vec)) => {
                let head = postcard::to_vec::<ClientHeader, 32>(self)
                    .expect("Failed to serialize client header");
                let mut full_data = Vec::with_capacity(head.len() + vec.len());
                full_data.extend_from_slice(&head);
                full_data.extend_from_slice(&vec);
                Bytes::from_owner(full_data)
            }
            Some(MessageData::Stack(vec)) => {
                let mut head = postcard::to_vec::<ClientHeader, HEAPLESS_SIZE>(self)
                    .expect("Failed to serialize client header");
                if head.len() + vec.len() <= HEAPLESS_SIZE {
                    head.extend_from_slice(&vec)
                        .expect("Failed to extend head with stack data");
                    return Bytes::from_owner(head);
                } else {
                    let mut full_data = Vec::with_capacity(head.len() + vec.len());
                    full_data.extend_from_slice(&head);
                    full_data.extend_from_slice(&vec);
                    Bytes::from_owner(full_data)
                }
            }
            None => {
                let head = postcard::to_vec::<ClientHeader, 32>(self)
                    .expect("Failed to serialize client header");
                Bytes::from_owner(head)
            }
        }
    }

    pub fn serialize_vec(&self, data: Option<MessageData>) -> Vec<u8> {
        let mut head = postcard::to_stdvec(self).expect("Failed to serialize client header");
        match data {
            Some(MessageData::Heap(vec)) => {
                head.extend_from_slice(&vec);
                head
            }
            Some(MessageData::Stack(vec)) => {
                head.extend_from_slice(&vec);
                head
            }
            None => head,
        }
    }
}

pub const HEAPLESS_SIZE: usize = 64;

pub enum MessageData {
    Heap(Vec<u8>),
    Stack(heapless::Vec<u8, HEAPLESS_SIZE>),
}

pub fn serialize_value_to_message<T: Serialize>(value: T) -> MessageData {
    let result = postcard::to_vec::<T, HEAPLESS_SIZE>(&value);
    match result {
        Ok(vec) => MessageData::Stack(vec),
        Err(postcard::Error::SerializeBufferFull) => {
            MessageData::Heap(postcard::to_stdvec(&value).expect("Failed to serialize value"))
        }
        Err(e) => panic!("Serialize error: {}", e),
    }
}

pub fn ser_server_value(header: ServerHeader, value: &Bytes) -> Bytes {
    let head = postcard::to_vec::<ServerHeader, HEAPLESS_SIZE>(&header)
        .expect("Failed to serialize server header");

    let mut data = Vec::with_capacity(head.len() + value.len());
    data.extend_from_slice(&head);
    data.extend_from_slice(value);

    Bytes::from_owner(data)
}

#[inline]
pub fn deserialize<T>(data: &[u8]) -> Result<T, String>
where
    T: for<'a> Deserialize<'a>,
{
    postcard::from_bytes(data).map_err(|e| e.to_string())
}

#[inline]
pub fn deserialize_from<T>(data: &[u8]) -> Result<(T, &[u8]), String>
where
    T: for<'a> Deserialize<'a>,
{
    postcard::take_from_bytes(data).map_err(|e| e.to_string())
}

#[inline]
pub fn deserialize_value<T>(data: &[u8]) -> Option<(T, usize)>
where
    T: for<'a> Deserialize<'a>,
{
    let (value, new_data) = postcard::take_from_bytes::<T>(data).ok()?;
    Some((value, data.len() - new_data.len()))
}