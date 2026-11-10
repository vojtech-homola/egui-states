//! The server's side of the wire format: encoding its messages and
//! decoding the client's.

use serde::{Deserialize, Serialize};

use crate::protocol::fastvec::{FastVec, serialize_to_data};
use crate::protocol::header::{ClientHeader, ServerHeader};

/// Chunk size of bulk transfers: Data, takes, image sets and collection
/// replacements. Much smaller than `MSG_SIZE_THRESHOLD`, the cap for merging
/// small messages into one frame, because the WebSocket library moves the
/// unsent rest of a message after every partial socket write, which costs
/// time quadratic in the message size.
pub(crate) const TRANSFER_CHUNK_SIZE: usize = 512 * 1024;

impl ServerHeader {
    pub fn serialize_value<const N: usize>(
        id: u64,
        type_id: u32,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, &'static str> {
        let header = ServerHeader::Value(id, type_id, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data).map_err(|_| "Failed to serialize header")?;
        data.extend_from_slice(value_data);
        Ok(data)
    }

    pub fn serialize_static<const N: usize>(
        id: u64,
        type_id: u32,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, &'static str> {
        let header = ServerHeader::Static(id, type_id, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data).map_err(|_| "Failed to serialize header")?;
        data.extend_from_slice(value_data);
        Ok(data)
    }

    pub fn serialize_value_take<const N: usize>(
        id: u64,
        type_id: u32,
        blocking: bool,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, &'static str> {
        let header =
            ServerHeader::ValueTake(id, type_id, blocking, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data).map_err(|_| "Failed to serialize header")?;
        data.extend_from_slice(value_data);
        Ok(data)
    }
}

impl ClientHeader {
    /// Bytes of payload that follow this header in the frame.
    pub(crate) fn payload_len(&self) -> u32 {
        match self {
            Self::Value(.., size) | Self::Signal(.., size) | Self::Message(size) => *size,
            Self::Ack(_) | Self::Handshake(..) => 0,
        }
    }

    #[inline]
    pub fn deserialize(msg: &[u8]) -> Result<(Self, usize), ()> {
        let (header, rest) = postcard::take_from_bytes::<Self>(msg).map_err(|_| ())?;
        Ok((header, msg.len() - rest.len()))
    }
}

impl<const N: usize> FastVec<N> {
    #[inline]
    pub(crate) fn new_heap() -> Self {
        Self::Heap(Vec::new())
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
}

#[inline]
pub(crate) fn deserialize_value<T>(data: &[u8]) -> Result<(T, usize), ()>
where
    T: for<'a> Deserialize<'a>,
{
    let (value, new_data) = postcard::take_from_bytes::<T>(data).map_err(|_| ())?;
    Ok((value, data.len() - new_data.len()))
}

#[inline]
pub(crate) fn serialize_heap<T, const N: usize>(value: &T) -> Result<FastVec<N>, ()>
where
    T: Serialize,
{
    postcard::serialize_with_flavor::<T, FastVec<N>, FastVec<N>>(value, FastVec::new_heap())
        .map_err(|_| ())
}
