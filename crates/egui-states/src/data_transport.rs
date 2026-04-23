use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use crate::serialization::{FastVec, ServerHeader, serialize, serialize_heap};

#[derive(Serialize, Deserialize, PartialEq, Clone, Copy, Debug)]
pub(crate) enum DataType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
}

impl DataType {
    pub(crate) fn element_size(&self) -> usize {
        match self {
            DataType::U8 | DataType::I8 => 1,
            DataType::U16 | DataType::I16 => 2,
            DataType::U32 | DataType::I32 | DataType::F32 => 4,
            DataType::U64 | DataType::I64 | DataType::F64 => 8,
        }
    }

    #[cfg(feature = "build_scripts")]
    pub(crate) fn get_id(&self) -> u8 {
        match self {
            DataType::U8 => 0,
            DataType::U16 => 1,
            DataType::U32 => 2,
            DataType::U64 => 3,
            DataType::I8 => 4,
            DataType::I16 => 5,
            DataType::I32 => 6,
            DataType::I64 => 7,
            DataType::F32 => 8,
            DataType::F64 => 9,
        }
    }

    #[cfg(feature = "server")]
    pub(crate) fn from_id(id: u8) -> Result<Self, ()> {
        match id {
            0 => Ok(DataType::U8),
            1 => Ok(DataType::U16),
            2 => Ok(DataType::U32),
            3 => Ok(DataType::U64),
            4 => Ok(DataType::I8),
            5 => Ok(DataType::I16),
            6 => Ok(DataType::I32),
            7 => Ok(DataType::I64),
            8 => Ok(DataType::F32),
            9 => Ok(DataType::F64),
            _ => Err(()),
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

impl DataHeader {
    #[cfg(feature = "server")]
    #[inline]
    pub(crate) fn serialize(self, id: u64, heap: bool) -> Result<FastVec<32>, ()> {
        let header = ServerHeader::Data(id, self);
        match heap {
            true => serialize_heap(&header).map_err(|_| ()),
            false => serialize(&header).map_err(|_| ()),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum MultiDataHeader {
    Remove(u32, bool),
    Modify(u32, DataHeader),
    Reset(bool),
}
