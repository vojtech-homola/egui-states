use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) enum MapHeader {
    All(u64),
    Set,
    Remove,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum VecHeader {
    All(u64),
    Set(u64),
    Add,
    Remove(u64),
}
