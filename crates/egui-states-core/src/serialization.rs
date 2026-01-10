use bytes::Bytes;
use postcard::ser_flavors::Flavor;
use serde::{Deserialize, Serialize};

use crate::nohash::NoHashMap;

use crate::collections::{ListHeader, MapHeader};
use crate::graphs::GraphHeader;
use crate::image::ImageHeader;

pub struct StackVec<const N: usize>([u8; N], usize);

impl<const N: usize> AsRef<[u8]> for StackVec<N> {
    fn as_ref(&self) -> &[u8] {
        &self.0[..self.1]
    }
}

pub enum FastVec<const N: usize> {
    Heap(Vec<u8>),
    Stack(StackVec<N>),
}

impl<const N: usize> FastVec<N> {
    #[inline]
    pub fn new() -> Self {
        Self::Stack(StackVec([0; N], 0))
    }

    #[inline]
    pub fn to_bytes(self) -> Bytes {
        match self {
            Self::Heap(vec) => Bytes::from_owner(vec),
            Self::Stack(vec) => Bytes::from_owner(vec),
        }
    }

    #[inline]
    pub fn to_vec(self) -> Vec<u8> {
        match self {
            Self::Heap(vec) => vec,
            Self::Stack(stack_vec) => stack_vec.0[..stack_vec.1].to_vec(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Heap(vec) => vec.len(),
            Self::Stack(stack_vec) => stack_vec.1,
        }
    }

    pub fn extend_from_slice(&mut self, data: &[u8]) {
        match self {
            Self::Heap(vec) => {
                vec.extend_from_slice(data);
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 + data.len() <= N {
                    stack_vec.0[stack_vec.1..stack_vec.1 + data.len()].copy_from_slice(data);
                    stack_vec.1 += data.len();
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + data.len());
                    new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                    new_vec.extend_from_slice(data);
                    *self = Self::Heap(new_vec);
                }
            }
        }
    }

    pub fn extend_from_data<const M: usize>(&mut self, data: &FastVec<M>) {
        match self {
            Self::Heap(vec) => match data {
                FastVec::Heap(dvec) => vec.extend_from_slice(dvec),
                FastVec::Stack(dvec) => vec.extend_from_slice(dvec.as_ref()),
            },
            Self::Stack(stack_vec) => match data {
                FastVec::Heap(dvec) => {
                    if stack_vec.1 + dvec.len() <= N {
                        stack_vec.0[stack_vec.1..stack_vec.1 + dvec.len()].copy_from_slice(&dvec);
                        stack_vec.1 += dvec.len();
                    } else {
                        let mut new_vec = Vec::with_capacity(stack_vec.1 + dvec.len());
                        new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                        new_vec.extend_from_slice(&dvec);
                        *self = Self::Heap(new_vec);
                    }
                }
                FastVec::Stack(dvec) => {
                    if stack_vec.1 + dvec.1 <= N {
                        stack_vec.0[stack_vec.1..stack_vec.1 + dvec.1]
                            .copy_from_slice(&dvec.0[..dvec.1]);
                        stack_vec.1 += dvec.1;
                    } else {
                        let mut new_vec = Vec::with_capacity(stack_vec.1 + dvec.1);
                        new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                        new_vec.extend_from_slice(&dvec.0[..dvec.1]);
                        *self = Self::Heap(new_vec);
                    }
                }
            },
        }
    }
}

impl<const N: usize> Flavor for FastVec<N> {
    type Output = Self;

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        match self {
            Self::Heap(vec) => {
                vec.push(data);
                Ok(())
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 < N {
                    stack_vec.0[stack_vec.1] = data;
                    stack_vec.1 += 1;
                    Ok(())
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + 1);
                    new_vec.extend_from_slice(&stack_vec.0);
                    new_vec.push(data);
                    *self = Self::Heap(new_vec);
                    Ok(())
                }
            }
        }
    }

    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        match self {
            Self::Heap(vec) => {
                vec.extend_from_slice(data);
                Ok(())
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 + data.len() <= N {
                    stack_vec.0[stack_vec.1..stack_vec.1 + data.len()].copy_from_slice(data);
                    stack_vec.1 += data.len();
                    Ok(())
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + data.len());
                    new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                    new_vec.extend_from_slice(data);
                    *self = Self::Heap(new_vec);
                    Ok(())
                }
            }
        }
    }

    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(self)
    }
}

pub const HEAPLESS_SIZE: usize = 32;
pub type MessageData = FastVec<HEAPLESS_SIZE>;

#[derive(Serialize, Deserialize)]
pub enum ServerHeader {
    Value(u64, bool),
    Static(u64, bool),
    Image(u64, bool, ImageHeader),
    Graph(u64, bool, GraphHeader),
    List(u64, bool, ListHeader),
    Map(u64, bool, MapHeader),
    Update(f32),
}

impl ServerHeader {
    pub fn serialize_to_slice<'a>(&self, buffer: &'a mut [u8]) -> &'a [u8] {
        postcard::to_slice::<ServerHeader>(self, buffer).expect("Failed to serialize server header")
    }

    pub fn serialize_to_bytes(&self) -> Bytes {
        let result = postcard::to_vec::<ServerHeader, HEAPLESS_SIZE>(self);
        match result {
            Ok(vec) => Bytes::from_owner(vec),
            Err(postcard::Error::SerializeBufferFull) => Bytes::from_owner(
                postcard::to_stdvec(self).expect("Failed to serialize server header"),
            ),
            Err(e) => panic!("Serialize error: {}", e),
        }
    }

    pub fn serialize_to_bytes_data(&self, data: Option<Bytes>) -> Bytes {
        match data {
            Some(b) => {
                let mut head = postcard::to_vec::<ServerHeader, HEAPLESS_SIZE>(self)
                    .expect("Failed to serialize server header");

                if b.len() + head.len() <= HEAPLESS_SIZE {
                    head.extend_from_slice(&b)
                        .expect("Failed to extend head with stack data");
                    Bytes::from_owner(head)
                } else {
                    let mut full_data = Vec::with_capacity(head.len() + b.len());
                    full_data.extend_from_slice(&head);
                    full_data.extend_from_slice(&b);
                    Bytes::from_owner(full_data)
                }
            }
            None => {
                let head = postcard::to_vec::<ServerHeader, HEAPLESS_SIZE>(self)
                    .expect("Failed to serialize server header");
                Bytes::from_owner(head)
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum ClientHeader {
    Value(u64, bool, u32),
    Signal(u64, u32),
    Ack(u64),
    Error(String),
    Handshake(u16, u64, NoHashMap<u64, u64>),
}

impl ClientHeader {
    pub fn serialize_handshake(
        protocol: u16,
        version: u64,
        types: NoHashMap<u64, u64>,
    ) -> FastVec<64> {
        let header = ClientHeader::Handshake(protocol, version, types);
        let data = postcard::to_stdvec(&header).expect("Failed to serialize handshake");
        FastVec::Heap(data)
    }

    pub fn deserialize(msg: &[u8]) -> Result<(Self, usize), ()> {
        let (header, rest) = postcard::take_from_bytes::<ClientHeader>(msg).map_err(|_| ())?;
        Ok((header, msg.len() - rest.len()))
    }
}

pub fn to_message_data<T: Serialize>(value: &T, data: Option<MessageData>) -> MessageData {
    let mut new_data =
        postcard::serialize_with_flavor::<T, MessageData, MessageData>(value, MessageData::new())
            .expect("Failed to serialize value");

    if let Some(d) = data {
        new_data.extend_from_data(&d);
    }

    new_data
}

#[inline]
pub fn to_message<T: Serialize>(value: T) -> MessageData {
    postcard::serialize_with_flavor::<T, MessageData, MessageData>(&value, MessageData::new())
        .expect("Failed to serialize value")
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
pub fn deserialize_value<T>(data: &[u8]) -> Result<(T, usize), ()>
where
    T: for<'a> Deserialize<'a>,
{
    let (value, new_data) = postcard::take_from_bytes::<T>(data).map_err(|_| ())?;
    Ok((value, data.len() - new_data.len()))
}

pub fn serialize_to_data<T, const N: usize>(value: &T, data: FastVec<N>) -> FastVec<N>
where
    T: Serialize,
{
    postcard::serialize_with_flavor::<T, FastVec<N>, FastVec<N>>(value, data)
        .expect("Failed to serialize value")
}
