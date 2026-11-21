//! The client's side of the wire format: decoding the server's messages
//! and encoding its own.

use serde::{Deserialize, Serialize};

use crate::protocol::collections::{MapHeader, VecHeader};
use crate::protocol::data::{DataMultiTakeHeader, MultiDataHeader};
use crate::protocol::fastvec::FastVec;
use crate::protocol::header::{ClientHeader, ServerHeader};
use crate::protocol::image::ImageMultiHeader;

impl ServerHeader {
    /// Bytes of payload that follow this header in the frame.
    pub(crate) fn payload_len(&self) -> u32 {
        match self {
            Self::Value(.., size)
            | Self::ValueTake(.., size)
            | Self::Static(.., size)
            | Self::Image(_, _, size)
            | Self::ValueVec(.., size)
            | Self::ValueMap(.., size) => *size,
            Self::Data(_, header) => header.payload_len(),
            Self::DataTake(_, header, _) => header.payload_len(),
            Self::MultiData(_, MultiDataHeader::Modify(_, header)) => header.payload_len(),
            Self::DataMultiTake(_, DataMultiTakeHeader::Modify(_, header, _)) => {
                header.payload_len()
            }
            Self::ImageMulti(_, ImageMultiHeader::Modify(_, _, size)) => *size,
            Self::Transfer(.., size) => *size,
            Self::MultiData(..) | Self::DataMultiTake(..) | Self::ImageMulti(..) => 0,
            Self::Update(_)
            | Self::TransferLimits(..)
            | Self::LaneMark
            | Self::ConnectPending(_) => 0,
        }
    }

    /// Bytes of a typed header that precede the payload's elements or
    /// pixels.
    pub(crate) fn header_len(&self) -> u32 {
        match self {
            Self::Image(_, header, _) => header.header_len(),
            Self::ImageMulti(_, ImageMultiHeader::Modify(_, header, _)) => header.header_len(),
            Self::Data(_, header) => header.header_len(),
            Self::DataTake(_, header, _) => header.header_len(),
            Self::MultiData(_, MultiDataHeader::Modify(_, header)) => header.header_len(),
            Self::DataMultiTake(_, DataMultiTakeHeader::Modify(_, header, _)) => {
                header.header_len()
            }
            _ => 0,
        }
    }

    /// Whether applying the message changes a state the UI shows, so the
    /// client repaints after the frame that carried it. The first and middle
    /// chunks of a split transfer are only buffered and do not.
    pub(crate) fn changes_state(&self) -> bool {
        match self {
            Self::Value(..) | Self::ValueTake(..) | Self::Static(..) => true,
            Self::ValueVec(_, _, header, _) => {
                !matches!(header, VecHeader::Begin(_) | VecHeader::Chunk(_))
            }
            Self::ValueMap(_, _, header, _) => {
                !matches!(header, MapHeader::Begin(_) | MapHeader::Chunk(_))
            }
            Self::Image(_, header, _) => header.changes_state(),
            Self::ImageMulti(_, header) => header.changes_state(),
            Self::Data(_, header) => header.changes_state(),
            Self::DataTake(_, header, _) => header.is_terminal(),
            Self::MultiData(_, header) => match header {
                MultiDataHeader::Remove(_) | MultiDataHeader::Reset => true,
                MultiDataHeader::Modify(_, header) => header.changes_state(),
            },
            Self::DataMultiTake(_, header) => match header {
                DataMultiTakeHeader::Remove(_) | DataMultiTakeHeader::Reset => true,
                DataMultiTakeHeader::Modify(_, header, _) => header.is_terminal(),
            },
            Self::Update(_)
            | Self::Transfer(..)
            | Self::TransferLimits(..)
            | Self::LaneMark
            | Self::ConnectPending(_) => false,
        }
    }

    /// The parse error for a frame that ends inside this message's payload.
    pub(crate) fn incomplete(&self) -> &'static str {
        match self {
            Self::Value(..) => "Incomplete data for Value message",
            Self::Static(..) => "Incomplete data for Static message",
            Self::ValueTake(..) => "Incomplete data for ValueTake message",
            Self::ValueVec(..) => "Incomplete data for ValueVec message",
            Self::ValueMap(..) => "Incomplete data for Map message",
            Self::Image(..) | Self::ImageMulti(..) => "Incomplete data for Image message",
            Self::Data(..) | Self::MultiData(..) => "Incomplete data for Data/DataStatic message",
            Self::DataTake(..) | Self::DataMultiTake(..) => "Incomplete data for DataTake message",
            Self::Update(_) => "Incomplete Update message",
            Self::Transfer(..) => "Incomplete data for Transfer message",
            Self::TransferLimits(..) => "Incomplete TransferLimits message",
            Self::LaneMark => "Incomplete LaneMark message",
            Self::ConnectPending(_) => "Incomplete ConnectPending message",
        }
    }

    #[inline]
    pub fn deserialize(msg: &[u8]) -> Result<(Self, usize), ()> {
        let (header, rest) = postcard::take_from_bytes::<Self>(msg).map_err(|_| ())?;
        Ok((header, msg.len() - rest.len()))
    }
}

// ClientHeader ---------------------------------------------------------------
// ----------------------------------------------------------------------------
pub type MessageData = FastVec<32>;

impl ClientHeader {
    pub fn serialize_handshake(
        protocol: u16,
        version: Option<u64>,
        token: Option<String>,
    ) -> FastVec<64> {
        let header = ClientHeader::Handshake(protocol, version, token);
        let data = postcard::to_stdvec(&header).expect("Failed to serialize handshake");
        FastVec::Heap(data)
    }
}

// Message serialization ---------------------------------------------------------------
// -------------------------------------------------------------------------------------
#[inline]
pub(crate) fn to_message<T: Serialize>(value: T) -> MessageData {
    postcard::serialize_with_flavor::<T, MessageData, MessageData>(&value, MessageData::new())
        .expect("Failed to serialize value")
}

#[inline]
pub(crate) fn deserialize<T>(data: &[u8]) -> Result<T, String>
where
    T: for<'a> Deserialize<'a>,
{
    postcard::from_bytes(data).map_err(|e| e.to_string())
}

pub(crate) struct Deserializer<'a> {
    data: &'a [u8],
}

impl<'a> Deserializer<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub(crate) fn get<T: for<'b> Deserialize<'b>>(&mut self) -> Result<T, String> {
        let (value, new_data) =
            postcard::take_from_bytes::<T>(self.data).map_err(|e| e.to_string())?;
        self.data = new_data;
        Ok(value)
    }
}
