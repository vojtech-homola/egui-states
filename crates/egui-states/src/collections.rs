use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) enum MapHeader {
    All,
    Set,
    Remove,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ListHeader {
    All,
    Set(u64),
    Add,
    Remove(u64),
}
