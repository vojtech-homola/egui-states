use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

// use egui_states_core::serialization::{TYPE_DICT, serialize_vec};
use egui_states_core::nohash::NoHashMap;
use egui_states_core::values::ObjectType;

use crate::sender::MessageSender;
use crate::server::SyncTrait;

pub(crate) trait MapTrait: Send + Sync {
    fn get_type(&self) -> (ObjectType, ObjectType);
}

pub(crate) struct ValueMap {
    id: u64,
    value_type: (ObjectType, ObjectType),
    value_id: (u64, u64),
    map: RwLock<NoHashMap<Bytes, Bytes>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl SyncTrait for ValueMap {
    fn sync(&self) {}
}
