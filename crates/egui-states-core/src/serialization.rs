use bytes::Bytes;
use postcard::ser_flavors::Flavor;
use serde::{Deserialize, Serialize};

use crate::collections::{ListHeader, MapHeader};
use crate::controls::{ControlClient, ControlServer};
use crate::graphs::GraphHeader;
use crate::image::ImageHeader;

#[derive(Serialize, Deserialize)]
pub enum ServerHeader {
    Value(u64, bool),
    Static(u64, bool),
    Image(u64, bool, ImageHeader),
    Graph(u64, bool, GraphHeader),
    List(u64, bool, ListHeader),
    Map(u64, bool, MapHeader),
    Control(ControlServer),
}

impl ServerHeader {
    pub fn serialize_control(control: ControlServer) -> Bytes {
        let data = postcard::to_stdvec(&ServerHeader::Control(control))
            .expect("Failed to serialize server control header");
        Bytes::from_owner(data)
    }

    pub fn serialize_to_slice<'a>(&self, buffer: &'a mut [u8]) -> &'a [u8] {
        postcard::to_slice::<ServerHeader>(self, buffer)
            .expect("Failed to serialize server header")    
        }
}

#[derive(Serialize, Deserialize)]
pub enum ClientHeader {
    Value(u64, bool),
    Signal(u64),
    Control(ControlClient),
}

impl ClientHeader {
    pub fn ack(id: u64) -> Self {
        ClientHeader::Control(ControlClient::Ack(id))
    }

    pub fn error(message: String) -> Self {
        ClientHeader::Control(ControlClient::Error(message))
    }

    pub fn serialize_handshake(protocol: u64, version: u64) -> MessageData {
        let header = ClientHeader::Control(ControlClient::Handshake(protocol, version));
        let data =
            postcard::to_vec::<ClientHeader, HEAPLESS_SIZE>(&header).expect("Failed to serialize");
        MessageData::Stack(data)
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

    pub fn deserialize_control(data: &Bytes) -> Result<ControlClient, ()> {
        match postcard::from_bytes::<ClientHeader>(data) {
            Ok(ClientHeader::Control(control)) => Ok(control),
            _ => Err(()),
        }
    }

    pub fn serialize_message(&self, data: Option<MessageData>) -> MessageData {
        match self {
            ClientHeader::Control(ControlClient::TypesAnswer(_)) => {
                let mut head =
                    postcard::to_stdvec(self).expect("Failed to serialize client header");
                match data {
                    Some(MessageData::Heap(vec)) => {
                        head.extend_from_slice(&vec);
                        MessageData::Heap(head)
                    }
                    Some(MessageData::Stack(vec)) => {
                        head.extend_from_slice(&vec);
                        MessageData::Heap(head)
                    }
                    None => MessageData::Heap(head),
                }
            }
            ClientHeader::Control(ControlClient::Error(_)) => {
                let mut head =
                    postcard::to_stdvec(self).expect("Failed to serialize client header");
                match data {
                    Some(MessageData::Heap(vec)) => {
                        head.extend_from_slice(&vec);
                        MessageData::Heap(head)
                    }
                    Some(MessageData::Stack(vec)) => {
                        head.extend_from_slice(&vec);
                        MessageData::Heap(head)
                    }
                    None => MessageData::Heap(head),
                }
            }
            _ => match data {
                Some(MessageData::Heap(vec)) => {
                    let head = postcard::to_vec::<ClientHeader, 24>(self)
                        .expect("Failed to serialize client header");
                    let mut full_data = Vec::with_capacity(head.len() + vec.len());
                    full_data.extend_from_slice(&head);
                    full_data.extend_from_slice(&vec);
                    MessageData::Heap(full_data)
                }
                Some(MessageData::Stack(vec)) => {
                    let mut head = postcard::to_vec::<ClientHeader, HEAPLESS_SIZE>(self)
                        .expect("Failed to serialize client header");
                    if vec.len() + head.len() <= HEAPLESS_SIZE {
                        head.extend_from_slice(&vec)
                            .expect("Failed to extend head with stack data");
                        MessageData::Stack(head)
                    } else {
                        let mut full_data = Vec::with_capacity(head.len() + vec.len());
                        full_data.extend_from_slice(&head);
                        full_data.extend_from_slice(&vec);
                        MessageData::Heap(full_data)
                    }
                }
                None => {
                    let head = postcard::to_vec::<ClientHeader, HEAPLESS_SIZE>(self)
                        .expect("Failed to serialize client header");
                    MessageData::Stack(head)
                }
            },
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

pub enum SerResult {
    Ok(usize),
    Heap(Vec<u8>),
}

pub fn serialize_value_slice<T>(value: &T, buffer: &mut [u8]) -> SerResult
where
    T: Serialize,
{
    let original_len = buffer.len();
    match postcard::to_slice::<T>(value, buffer) {
        Ok(slice) => SerResult::Ok(original_len - slice.len()),
        Err(postcard::Error::SerializeBufferFull) => {
            let vec = postcard::to_stdvec(value).expect("Failed to serialize value");
            SerResult::Heap(vec)
        }
        Err(e) => panic!("Serialize error: {}", e),
    }
}

struct VecFlavor<'a>(&'a mut Vec<u8>);

impl<'a> VecFlavor<'a> {
    fn new(buffer: &'a mut Vec<u8>) -> Self {
        Self(buffer)
    }
}

impl Flavor for VecFlavor<'_> {
    type Output = ();

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.0.push(data);
        Ok(())
    }

    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        self.0.extend_from_slice(data);
        Ok(())
    }

    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(())
    }
}

pub fn serialize_value_vec<T>(value: &T, buffer: &mut Vec<u8>) -> bool
where
    T: Serialize,
{
    let buf = VecFlavor::new(buffer);
    let result = postcard::serialize_with_flavor::<T, VecFlavor, ()>(value, buf);

    result.is_ok()
}
