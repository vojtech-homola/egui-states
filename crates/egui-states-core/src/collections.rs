use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum MapHeader {
    All,
    Set,
    Remove,
}

#[derive(Serialize, Deserialize)]
pub enum ListHeader {
    All,
    Set(u64),
    Add,
    Remove(u64),
}
