use serde::{Deserialize, Serialize};

use crate::typed::ObjectType;

#[cfg(any(feature = "server", feature = "python"))]
use crate::protocol::{FastVec, ServerHeader, serialize, serialize_heap};

/// The type id a Data header of type `object_type` carries on the wire:
/// `None` for [`ObjectType::Empty`], which means the state has no header, the
/// type's hash otherwise.
///
/// Both peers derive it here, so they agree whether a state has a header and
/// which type it carries.
pub(crate) fn header_type_id(object_type: &ObjectType) -> Option<u32> {
    match object_type {
        ObjectType::Empty => None,
        object_type => Some(object_type.get_hash()),
    }
}

/// Element type of a numeric buffer.
///
/// The discriminants are the type ids every front-end uses, and the variant
/// order is the wire tag; both are part of the protocol.
#[derive(Serialize, Deserialize, PartialEq, Clone, Copy, Debug)]
#[repr(u8)]
pub(crate) enum DataType {
    U8 = 0,
    U16 = 1,
    U32 = 2,
    U64 = 3,
    I8 = 4,
    I16 = 5,
    I32 = 6,
    I64 = 7,
    F32 = 8,
    F64 = 9,
}

impl DataType {
    /// The protocol type id.
    #[cfg(any(feature = "client", feature = "server", test))]
    pub(crate) const fn id(self) -> u8 {
        self as u8
    }

    pub(crate) fn item_size(&self) -> usize {
        match self {
            DataType::U8 | DataType::I8 => 1,
            DataType::U16 | DataType::I16 => 2,
            DataType::U32 | DataType::I32 | DataType::F32 => 4,
            DataType::U64 | DataType::I64 | DataType::F64 => 8,
        }
    }

    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn from_id(id: u8) -> Result<Self, ()> {
        Self::ALL.get(usize::from(id)).copied().ok_or(())
    }

    /// Every type, in id order.
    #[cfg(any(test, feature = "server", feature = "python"))]
    const ALL: [Self; 10] = [
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::F32,
        Self::F64,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_types_are_listed_in_id_order() {
        for (index, data_type) in DataType::ALL.iter().enumerate() {
            assert_eq!(usize::from(data_type.id()), index);
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) enum TransportType {
    Set(u64),          // element count of data
    Add(u64),          // element count of data
    Replace(u64, u64), // start index and element count of data
}

/// A typed header sent with a Data message. Its `size` encoded bytes come
/// first in the payload, before any elements.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeaderBytes {
    /// The type hash of the header type.
    pub(crate) type_id: u32,
    pub(crate) size: u32,
}

/// What a Data message does to the state's header.
///
/// A message that creates the data of a state with a header type always
/// sets one; a state without a header type never gets one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub(crate) enum HeaderUpdate {
    /// The header stays as it is.
    Keep,
    /// The header becomes the one encoded in the payload.
    Set(HeaderBytes),
}

#[cfg(feature = "client")]
impl HeaderUpdate {
    /// The header bytes this update puts in front of the elements.
    pub(crate) fn len(&self) -> u32 {
        match self {
            Self::Set(header) => header.size,
            Self::Keep => 0,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum DataHeader {
    /// Data type, transport type, element bytes, header.
    All(DataType, TransportType, u32, HeaderUpdate),
    StartBatch(u64, u32), // total element count of data, size of first batch
    Batch(u32),           // size of batch
    /// Data type, transport type, element bytes of the last batch, header.
    End(DataType, TransportType, u32, HeaderUpdate),
    /// Start and element count to drain, header.
    Drain(u64, u64, HeaderUpdate),
    /// Header.
    Clear(HeaderUpdate),
    /// A new header alone.
    Header(HeaderBytes),
    /// The data and its header return to "not set". Only a `Data` receives
    /// it; a `DataMulti` removes a key instead.
    Reset,
}

#[cfg(any(feature = "server", feature = "python"))]
impl DataHeader {
    pub(crate) fn serialize(self, id: u64, heap: bool) -> Result<FastVec<32>, ()> {
        let header = ServerHeader::Data(id, self);
        match heap {
            true => serialize_heap(&header).map_err(|_| ()),
            false => serialize(&header).map_err(|_| ()),
        }
    }
}

#[cfg(feature = "client")]
impl DataHeader {
    /// Bytes of elements in the payload, after the header bytes.
    pub(crate) fn payload_len(&self) -> u32 {
        match self {
            Self::All(_, _, size, _) | Self::End(_, _, size, _) => *size,
            Self::StartBatch(_, size) | Self::Batch(size) => *size,
            Self::Drain(..) | Self::Clear(..) | Self::Header(..) | Self::Reset => 0,
        }
    }

    /// Bytes of the header at the start of the payload.
    pub(crate) fn header_len(&self) -> u32 {
        match self {
            Self::All(.., header) | Self::End(.., header) => header.len(),
            Self::Drain(.., header) | Self::Clear(header) => header.len(),
            Self::Header(header) => header.size,
            Self::StartBatch(..) | Self::Batch(_) | Self::Reset => 0,
        }
    }

    /// Whether applying the message changes the data; the first and middle
    /// chunks of a batch are only buffered.
    pub(crate) fn changes_state(&self) -> bool {
        !matches!(self, Self::StartBatch(..) | Self::Batch(_))
    }

    /// Whether the client acknowledges this message, also when it fails.
    pub(crate) fn requires_ack(&self) -> bool {
        matches!(self, Self::All(..) | Self::End(..) | Self::Drain(..))
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum DataTakeHeader {
    /// Data type, element count, element bytes, header (`None` for a state
    /// without a header type).
    All(DataType, u64, u32, Option<HeaderBytes>),
    StartBatch(u64, u32), // total element count, size of first batch
    Batch(u32),           // size of batch
    /// Data type, element count, element bytes of the last batch, header
    /// (`None` for a state without a header type).
    End(DataType, u64, u32, Option<HeaderBytes>),
}

#[cfg(any(feature = "server", feature = "python"))]
impl DataTakeHeader {
    pub(crate) fn serialize(self, id: u64, blocking: bool, heap: bool) -> Result<FastVec<32>, ()> {
        let header = ServerHeader::DataTake(id, self, blocking);
        match heap {
            true => serialize_heap(&header).map_err(|_| ()),
            false => serialize(&header).map_err(|_| ()),
        }
    }
}

#[cfg(feature = "client")]
impl DataTakeHeader {
    /// Bytes of elements in the payload, after the header bytes.
    pub(crate) fn payload_len(&self) -> u32 {
        match self {
            Self::All(_, _, size, _) | Self::End(_, _, size, _) => *size,
            Self::StartBatch(_, size) | Self::Batch(size) => *size,
        }
    }

    /// Bytes of the header at the start of the payload.
    pub(crate) fn header_len(&self) -> u32 {
        match self {
            Self::All(.., header) | Self::End(.., header) => header.map_or(0, |h| h.size),
            Self::StartBatch(..) | Self::Batch(_) => 0,
        }
    }

    /// Whether the message completes a transfer.
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(self, Self::All(..) | Self::End(..))
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum MultiDataHeader {
    Remove(u32),             // remove index from data collection
    Modify(u32, DataHeader), // modify index in data collection
    Reset,                   // reset data collection to empty
}

#[cfg(any(feature = "server", feature = "python"))]
impl MultiDataHeader {
    pub(crate) fn serialize_modify(
        id: u64,
        index: u32,
        header: DataHeader,
    ) -> Result<FastVec<32>, ()> {
        let header = ServerHeader::MultiData(id, MultiDataHeader::Modify(index, header));
        serialize_heap(&header).map_err(|_| ())
    }

    pub(crate) fn serialize(self, id: u64) -> Result<FastVec<32>, ()> {
        let message = ServerHeader::MultiData(id, self);
        serialize(&message).map_err(|_| ())
    }
}

#[cfg(feature = "client")]
impl MultiDataHeader {
    /// The key to acknowledge for this message, if the client acknowledges
    /// it.
    pub(crate) fn ack_key(&self) -> Option<u32> {
        match self {
            Self::Modify(key, header) if header.requires_ack() => Some(*key),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum DataMultiTakeHeader {
    Remove(u32),                       // remove index from data collection
    Modify(u32, DataTakeHeader, bool), // modify index in data collection, blocking flag
    Reset,                             // reset data collection to empty
}

#[cfg(feature = "client")]
impl DataMultiTakeHeader {
    /// Whether a failure of this message must still be acknowledged: only a
    /// blocking transfer's last message waits for one.
    pub(crate) fn requires_ack_on_failure(&self) -> bool {
        match self {
            Self::Modify(_, header, blocking) => *blocking && header.is_terminal(),
            Self::Remove(_) | Self::Reset => false,
        }
    }
}

#[cfg(any(feature = "server", feature = "python"))]
impl DataMultiTakeHeader {
    pub(crate) fn serialize_modify(
        id: u64,
        index: u32,
        header: DataTakeHeader,
        blocking: bool,
    ) -> Result<FastVec<32>, ()> {
        let header =
            ServerHeader::DataMultiTake(id, DataMultiTakeHeader::Modify(index, header, blocking));
        serialize_heap(&header).map_err(|_| ())
    }

    pub(crate) fn serialize(self, id: u64) -> Result<FastVec<32>, ()> {
        let message = ServerHeader::DataMultiTake(id, self);
        serialize(&message).map_err(|_| ())
    }
}
