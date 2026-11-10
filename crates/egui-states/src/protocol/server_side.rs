//! The server's side of the wire format: encoding its messages and
//! decoding the client's.

use serde::{Deserialize, Serialize};

use crate::protocol::VALUE_MAX_SIZE;
use crate::protocol::fastvec::{FastVec, serialize_to_data};
use crate::protocol::header::{ClientHeader, ServerHeader};

/// Chunk size of bulk transfers: Data, takes, image sets and collection
/// replacements. Much smaller than `MSG_SIZE_THRESHOLD`, the cap for merging
/// small messages into one frame, because the WebSocket library moves the
/// unsent rest of a message after every partial socket write, which costs
/// time quadratic in the message size.
pub(crate) const TRANSFER_CHUNK_SIZE: usize = 512 * 1024;

/// The longest encoding of a Data message's server header; a test checks the
/// worst case of every variant.
pub(crate) const MAX_SERVER_HEADER_LEN: usize = 64;

// The message that completes a transfer carries a chunk of elements and a
// typed header of up to a value's size; it must fit one WebSocket frame.
const _: () = assert!(
    TRANSFER_CHUNK_SIZE + VALUE_MAX_SIZE + MAX_SERVER_HEADER_LEN
        <= crate::protocol::WEBSOCKET_MAX_SIZE
);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::data::{
        DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, HeaderBytes, HeaderUpdate,
        MultiDataHeader, TransportType,
    };
    use crate::protocol::fastvec::serialize;

    #[test]
    fn every_data_header_fits_the_reserved_length() {
        let header = HeaderBytes {
            type_id: u32::MAX,
            size: u32::MAX,
        };
        let set = HeaderUpdate::Set(header);
        let transport = TransportType::Replace(u64::MAX, u64::MAX);
        let data = || {
            [
                DataHeader::All(DataType::F64, transport, true, u32::MAX, set),
                DataHeader::StartBatch(u64::MAX, u32::MAX),
                DataHeader::Batch(u32::MAX),
                DataHeader::End(DataType::F64, transport, true, u32::MAX, set),
                DataHeader::Drain(u64::MAX, u64::MAX, true, set),
                DataHeader::Clear(true, set),
                DataHeader::Header(true, header),
            ]
        };
        let take = || {
            [
                DataTakeHeader::All(DataType::F64, u64::MAX, true, u32::MAX, Some(header)),
                DataTakeHeader::StartBatch(u64::MAX, u32::MAX),
                DataTakeHeader::Batch(u32::MAX),
                DataTakeHeader::End(DataType::F64, u64::MAX, true, u32::MAX, Some(header)),
            ]
        };
        let mut messages = Vec::new();
        messages.extend(data().map(|header| ServerHeader::Data(u64::MAX, header)));
        messages.extend(data().map(|header| {
            ServerHeader::MultiData(u64::MAX, MultiDataHeader::Modify(u32::MAX, header))
        }));
        messages.extend(take().map(|header| ServerHeader::DataTake(u64::MAX, header, true)));
        messages.extend(take().map(|header| {
            ServerHeader::DataMultiTake(
                u64::MAX,
                DataMultiTakeHeader::Modify(u32::MAX, header, true),
            )
        }));
        assert_eq!(messages.len(), 22);
        for message in &messages {
            let len = serialize::<_, 128>(message).unwrap().len();
            assert!(len <= MAX_SERVER_HEADER_LEN, "{len} bytes");
        }
    }
}
