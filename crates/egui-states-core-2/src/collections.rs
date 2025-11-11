use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum MapMessageHeader {
    All(u64, u64),
    Set(u64, u64),
    Remove(u64),
}

#[derive(Serialize, Deserialize)]
pub enum ListMessageHeader {
    All(u64),
    Set(u64),
    Add(u64),
    Remove(u64),
}
