use parking_lot::RwLock;
use std::mem::size_of;
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::graphs::{GraphHeader, GraphType, GraphTyped};
use egui_states_core::nohash::NoHashMap;

use crate::sender::MessageSender;
use crate::server::{EnableTrait, SyncTrait};

pub(crate) struct GraphData {
    pub graph_type: GraphType,
    pub y: *const u8,
    pub x: Option<*const u8>,
    pub size: usize,
}

pub(crate) struct ValueGraphs {
    id: u64,
    graphs: RwLock<NoHashMap<u16, GraphTyped>>,
    graph_type: GraphType,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    enabled: AtomicBool,
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
            enabled: AtomicBool::new(false),
        })
    }

    pub(crate) fn set(&self, idx: u16, graph_data: GraphData, update: bool) {
        let graph = data_to_graph(&graph_data);

        let mut w = self.graphs.write();
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let data = graph.to_data(self.id, idx, update, None);
            self.sender.send(Bytes::from(data));
        }
        w.insert(idx, graph);
    }

    pub(crate) fn add_points(
        &self,
        idx: u16,
        graph_data: &GraphData,
        update: bool,
    ) -> Result<(), String> {
        let mut w = self.graphs.write();
        let graph = w
            .get_mut(&idx)
            .ok_or_else(|| "Graph index not found.".to_string())?;

        add_data_to_graph(&graph_data, graph);

        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let data = graph.to_data(
                self.id,
                idx,
                update,
                Some(graph_data.size / self.graph_type.bytes_size()),
            );
            self.sender.send(Bytes::from(data));
        }

        Ok(())
    }
}

fn add_data_to_graph(graph_data: &GraphData, graph: &mut GraphTyped) {
    let original_len = graph.y.len();
    graph.y.resize(original_len + graph_data.size, 0u8);
    unsafe {
        copy_nonoverlapping(
            graph_data.y,
            graph.y.as_mut_ptr().add(original_len),
            graph_data.size,
        );
    }

    if let Some(x_ptr) = graph_data.x {
        if let Some(x) = &mut graph.x {
            x.resize(original_len + graph_data.size, 0u8);
            unsafe {
                copy_nonoverlapping(x_ptr, x.as_mut_ptr().add(original_len), graph_data.size);
            }
        }
    }
}

fn data_to_graph(graph_data: &GraphData) -> GraphTyped {
    let mut y = Vec::with_capacity(graph_data.size);
    unsafe {
        copy_nonoverlapping(graph_data.y, y.as_mut_ptr(), graph_data.size);
        y.set_len(graph_data.size);
    }

    let x = match graph_data.x {
        Some(x_ptr) => {
            let mut x = Vec::with_capacity(graph_data.size);
            unsafe {
                copy_nonoverlapping(x_ptr, x.as_mut_ptr(), graph_data.size);
                x.set_len(graph_data.size);
            }
            Some(x)
        }
        None => None,
    };

    GraphTyped {
        graph_type: graph_data.graph_type,
        y,
        x,
    }
}
