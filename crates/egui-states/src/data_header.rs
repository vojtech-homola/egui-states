use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct DataHeaderAll {
    pub type_id: u32,
    pub update: bool,
    pub is_add: bool,
    pub header_size: u32,
    pub data_size: u32,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct DataHeaderHead {
    pub type_id: u32,
    pub data_size_all: u64,
    pub header_size: u32,
    pub data_size: u32,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct DataHeaderData {
    pub data_size: u32,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct DataHeaderEnd {
    pub update: bool,
    pub is_add: bool,
    pub data_size: u32,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum DataHeader {
    All(DataHeaderAll),
    Head(DataHeaderHead),
    Data(DataHeaderData),
    End(DataHeaderEnd),
}
