use parking_lot::RwLock;
use std::mem::size_of;
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::graphs::{GraphHeader, GraphType, GraphTyped};
use egui_states_core::nohash::NoHashMap;

use crate::python_convert::{FromPython, ToPython};
use crate::sender::MessageSender;
use crate::server::SyncTrait;

pub(crate) struct ValueGraphs {
    id: u64,
    graphs: RwLock<NoHashMap<u16, GraphTyped>>,
    graph_type: GraphType,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueGraphs {
    pub(crate) fn new(
        id: u64,
        sender: MessageSender,
        graph_type: GraphType,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            graphs: RwLock::new(NoHashMap::default()),
            graph_type,
            sender,
            connected,
        })
    }
}
