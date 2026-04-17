use postcard::ser_flavors::Flavor;
use serde::{Deserialize, Serialize};

use crate::collections::{MapHeader, VecHeader};
use crate::data_transport;
use crate::graphs::GraphHeader;
use crate::image_header::ImageHeader;

// TODO: make these constants configurable
pub(crate) const VALUE_MAX_SIZE: usize = 1024 * 1024; // 1 MB
pub(crate) const MSG_SIZE_THRESHOLD: usize = 1024 * 1024 * 10; // 10 MB
pub(crate) const MAX_MSG_COUNT: usize = 10;

pub(crate) struct StackVec<const N: usize>([u8; N], usize);

impl<const N: usize> AsRef<[u8]> for StackVec<N> {
    fn as_ref(&self) -> &[u8] {
        &self.0[..self.1]
    }
}

pub(crate) enum FastVec<const N: usize> {
    Heap(Vec<u8>),
    Stack(StackVec<N>),
}

impl<const N: usize> FastVec<N> {
    #[inline]
    pub fn new() -> Self {
        Self::Stack(StackVec([0; N], 0))
    }

    #[inline]
    pub(crate) fn new_heap() -> Self {
        Self::Heap(Vec::new())
    }

    #[cfg(feature = "server")]
    #[inline]
    pub fn from_vec(vec: Vec<u8>) -> Self {
        Self::Heap(vec)
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub fn to_bytes(self) -> bytes::Bytes {
        match self {
            Self::Heap(vec) => bytes::Bytes::from_owner(vec),
            Self::Stack(vec) => bytes::Bytes::from_owner(vec),
        }
    }

    #[cfg(target_arch = "wasm32")]
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

    pub(crate) fn reserve_exact(&mut self, additional: usize) {
        match self {
            Self::Heap(vec) => vec.reserve_exact(additional),
            Self::Stack(stack_vec) => {
                if stack_vec.1 + additional > N {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + additional);
                    new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                    *self = Self::Heap(new_vec);
                }
            }
        }
    }

    #[cfg(feature = "server")]
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

#[cfg(feature = "client")]
pub type MessageData = FastVec<32>;

#[derive(Serialize, Deserialize)]
pub(crate) enum ServerHeader {
    Value(u64, u32, bool, u32),
    ValueTake(u64, u32, bool, bool, u32),
    Static(u64, u32, bool, u32),
    Image(u64, bool, ImageHeader),
    Data(u64, data_transport::DataHeader),
    Graph(u64, bool, GraphHeader),
    ValueVec(u64, u32, bool, VecHeader, u32),
    ValueMap(u64, u32, bool, MapHeader, u32),
    Update(f32),
}

#[cfg(feature = "server")]
impl ServerHeader {
    pub fn serialize_to_slice<'a, 'b>(&'b self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        postcard::to_slice::<ServerHeader>(self, buffer).map_err(|_| ())
    }

    pub fn serialize_value<const N: usize>(
        id: u64,
        type_id: u32,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, ()> {
        let header = ServerHeader::Value(id, type_id, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data)?;
        data.extend_from_slice(value_data);
        Ok(data)
    }

    pub fn serialize_static<const N: usize>(
        id: u64,
        type_id: u32,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, ()> {
        let header = ServerHeader::Static(id, type_id, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data)?;
        data.extend_from_slice(value_data);
        Ok(data)
    }

    pub fn serialize_value_take<const N: usize>(
        id: u64,
        type_id: u32,
        blocking: bool,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, ()> {
        let header =
            ServerHeader::ValueTake(id, type_id, blocking, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data)?;
        data.extend_from_slice(value_data);
        Ok(data)
    }
}

#[cfg(feature = "client")]
impl ServerHeader {
    #[inline]
    pub fn deserialize(msg: &[u8]) -> Result<(Self, usize), ()> {
        let (header, rest) = postcard::take_from_bytes::<Self>(msg).map_err(|_| ())?;
        Ok((header, msg.len() - rest.len()))
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ClientHeader {
    Value(u64, u32, bool, u32),
    Signal(u64, u32, u32),
    Ack(u64),
    Handshake(u16, u64),
}

impl ClientHeader {
    #[cfg(feature = "client")]
    pub fn serialize_handshake(protocol: u16, version: u64) -> FastVec<64> {
        let header = ClientHeader::Handshake(protocol, version);
        let data = postcard::to_stdvec(&header).expect("Failed to serialize handshake");
        FastVec::Heap(data)
    }

    #[cfg(feature = "server")]
    #[inline]
    pub fn deserialize(msg: &[u8]) -> Result<(Self, usize), ()> {
        let (header, rest) = postcard::take_from_bytes::<Self>(msg).map_err(|_| ())?;
        Ok((header, msg.len() - rest.len()))
    }
}

#[cfg(feature = "client")]
#[inline]
pub(crate) fn to_message<T: Serialize>(value: T) -> MessageData {
    postcard::serialize_with_flavor::<T, MessageData, MessageData>(&value, MessageData::new())
        .expect("Failed to serialize value")
}

#[cfg(feature = "client")]
#[inline]
pub(crate) fn deserialize<T>(data: &[u8]) -> Result<T, String>
where
    T: for<'a> Deserialize<'a>,
{
    postcard::from_bytes(data).map_err(|e| e.to_string())
}

#[cfg(feature = "server")]
#[inline]
pub(crate) fn deserialize_value<T>(data: &[u8]) -> Result<(T, usize), ()>
where
    T: for<'a> Deserialize<'a>,
{
    let (value, new_data) = postcard::take_from_bytes::<T>(data).map_err(|_| ())?;
    Ok((value, data.len() - new_data.len()))
}

#[cfg(feature = "client")]
pub(crate) struct Deserializer<'a> {
    data: &'a [u8],
}

#[cfg(feature = "client")]
impl<'a> Deserializer<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub(crate) fn get<T: for<'b> Deserialize<'b>>(&mut self) -> Result<T, String> {
        let (value, new_data) =
            postcard::take_from_bytes::<T>(&self.data).map_err(|e| e.to_string())?;
        self.data = new_data;
        Ok(value)
    }
}

#[cfg(feature = "server")]
#[inline]
pub(crate) fn serialize<T, const N: usize>(value: &T) -> Result<FastVec<N>, ()>
where
    T: Serialize,
{
    postcard::serialize_with_flavor::<T, FastVec<N>, FastVec<N>>(value, FastVec::new())
        .map_err(|_| ())
}

#[cfg(feature = "server")]
#[inline]
pub(crate) fn serialize_heap<T, const N: usize>(value: &T) -> Result<FastVec<N>, ()>
where
    T: Serialize,
{
    postcard::serialize_with_flavor::<T, FastVec<N>, FastVec<N>>(value, FastVec::new_heap())
        .map_err(|_| ())
}

struct FastVecRef<'a, const N: usize>(&'a mut FastVec<N>);

impl<'a, const N: usize> Flavor for FastVecRef<'a, N> {
    type Output = ();

    #[inline]
    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.0.try_push(data)
    }

    #[inline]
    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        self.0.try_extend(data)
    }

    #[inline]
    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(())
    }
}

#[inline]
pub(crate) fn serialize_to_data<'a, T, const N: usize>(
    value: &T,
    data: &'a mut FastVec<N>,
) -> Result<(), ()>
where
    T: Serialize,
{
    let data_ref = FastVecRef(data);
    postcard::serialize_with_flavor::<T, FastVecRef<N>, ()>(value, data_ref).map_err(|_| ())
}
