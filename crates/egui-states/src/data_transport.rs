use serde::{Deserialize, Serialize};

#[cfg(any(feature = "server", feature = "python"))]
use crate::serialization::{FastVec, ServerHeader, serialize, serialize_heap};

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

#[derive(Serialize, Deserialize)]
pub(crate) enum TransportType {
    Set(u64),          // element count of data
    Add(u64),          // element count of data
    Replace(u64, u64), // start index and element count of data
}

#[derive(Serialize, Deserialize)]
pub(crate) enum DataHeader {
    All(DataType, TransportType, bool, u32), // data type, transport type, update flag, size of last batch
    StartBatch(u64, u32),                    // total element count of data, size of first batch
    Batch(u32),                              // size of batch
    End(DataType, TransportType, bool, u32), // data type, transport type, update flag, size of last batch
    Drain(u64, u64, bool), // start and element count of data to drain, update flag
    Clear(bool),           // update flag
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

#[derive(Serialize, Deserialize)]
pub(crate) enum DataTakeHeader {
    All(DataType, u64, bool, u32), // data type, element count, update flag, data size
    StartBatch(u64, u32),          // total element count, size of first batch
    Batch(u32),                    // size of batch
    End(DataType, u64, bool, u32), // data type, element count, update flag, size of last batch
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

#[derive(Serialize, Deserialize)]
pub(crate) enum MultiDataHeader {
    Remove(u32, bool),       // remove index from data collection
    Modify(u32, DataHeader), // modify index in data collection
    Reset(bool),             // reset data collection to empty
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

#[derive(Serialize, Deserialize)]
pub(crate) enum DataMultiTakeHeader {
    Remove(u32, bool),                 // remove index from data collection
    Modify(u32, DataTakeHeader, bool), // modify index in data collection, blocking flag
    Reset(bool),                       // reset data collection to empty
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
