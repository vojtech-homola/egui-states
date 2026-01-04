pub mod collections;
pub mod controls;
pub mod event_async;
pub mod graphs;
pub mod image;
pub mod nohash;
pub mod serialization;
pub mod types;

mod hasher;

use crate::hasher::StableHasher;
use std::hash::{Hash, Hasher};

pub const PROTOCOL_VERSION: u16 = 1;

pub fn generate_value_id(name: &str) -> u64 {
    let mut hasher = StableHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}
