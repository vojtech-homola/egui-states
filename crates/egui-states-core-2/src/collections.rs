use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum MapHeader {
    All(u64, u64),
    Set(u64, u64),
    Remove(u64),
}

#[derive(Serialize, Deserialize)]
pub enum ListHeader {
    All(u64),
    Set(u64),
    Add(u64),
    Remove(u64),
}
