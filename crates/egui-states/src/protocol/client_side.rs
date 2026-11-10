//! The client's side of the wire format: decoding the server's messages
//! and encoding its own.

use serde::{Deserialize, Serialize};

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
            Self::MultiData(..) | Self::DataMultiTake(..) | Self::ImageMulti(..) => 0,
            Self::Update(_) => 0,
        }
    }

    /// Bytes of a Data header that precede the payload's elements.
    pub(crate) fn header_len(&self) -> u32 {
        match self {
            Self::Data(_, header) => header.header_len(),
            Self::DataTake(_, header, _) => header.header_len(),
            Self::MultiData(_, MultiDataHeader::Modify(_, header)) => header.header_len(),
            Self::DataMultiTake(_, DataMultiTakeHeader::Modify(_, header, _)) => {
                header.header_len()
            }
            _ => 0,
        }
    }

    /// Whether the message asks the client to repaint once it is applied.
    pub(crate) fn update_flag(&self) -> bool {
        match self {
            Self::Value(_, _, update, _)
            | Self::ValueTake(_, _, _, update, _)
            | Self::Static(_, _, update, _)
            | Self::ValueVec(_, _, update, _, _)
            | Self::ValueMap(_, _, update, _, _) => *update,
            Self::Image(_, header, _) => header.update_flag(),
            Self::ImageMulti(_, header) => header.update_flag(),
            Self::Data(_, header) => header.update_flag(),
            Self::DataTake(_, header, _) => header.update_flag(),
            Self::MultiData(_, header) => match header {
                MultiDataHeader::Remove(_, update) | MultiDataHeader::Reset(update) => *update,
                MultiDataHeader::Modify(_, header) => header.update_flag(),
            },
            Self::DataMultiTake(_, header) => match header {
                DataMultiTakeHeader::Remove(_, update) | DataMultiTakeHeader::Reset(update) => {
                    *update
                }
                DataMultiTakeHeader::Modify(_, header, _) => header.update_flag(),
            },
            Self::Update(_) => false,
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
