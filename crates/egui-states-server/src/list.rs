use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

// use egui_states_core::serialization::{TYPE_LIST, serialize_vec};
use egui_states_core::values::ObjectType;

use crate::sender::MessageSender;
use crate::server::SyncTrait;

pub(crate) trait ListTrait: Send + Sync {
    fn get_type(&self) -> (ObjectType, ObjectType);
}

pub(crate) struct ValueList {
    id: u64,
    value_type: ObjectType,
    value_id: u64,
    list: RwLock<Vec<Bytes>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl SyncTrait for ValueList {
    fn sync(&self) {}
}
