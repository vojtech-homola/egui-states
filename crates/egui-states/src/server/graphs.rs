use parking_lot::RwLock;
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::graphs::{GraphDataInfo, GraphHeader, GraphType};
use crate::hashing::NoHashMap;
use crate::serialization::{ServerHeader, serialize};
use crate::server::sender::{MessageSender, SenderData};
use crate::server::server::{EnableTrait, SyncTrait};

pub(crate) struct GraphData {
    pub graph_type: GraphType,
    pub y: *const u8,
    pub x: Option<*const u8>,
    pub size: usize,
}

pub(crate) struct ValueGraphs {
    pub(crate) name: String,
    id: u64,
    graphs: RwLock<NoHashMap<u16, GraphTyped>>,
    graph_type: GraphType,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    enabled: AtomicBool,
}

impl ValueGraphs {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        graph_type: GraphType,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            graphs: RwLock::new(NoHashMap::default()),
            graph_type,
            sender,
            connected,
            enabled: AtomicBool::new(false),
        })
    }

    pub(crate) fn graph_type(&self) -> GraphType {
        self.graph_type
    }

    pub(crate) fn is_linear(&self, idx: u16) -> Result<bool, ()> {
        self.graphs
            .read()
            .get(&idx)
            .map_or(Err(()), |g| Ok(g.x.is_none()))
    }

    pub(crate) fn set(&self, idx: u16, graph_data: GraphData, update: bool) {
        let graph = data_to_graph(&graph_data);

        let mut w = self.graphs.write();
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let data = graph.to_data(self.id, idx, update, None);
            self.sender.send_single(SenderData::from_vec(data));
        }
        w.insert(idx, graph);
    }

    pub(crate) fn add_points(
        &self,
        idx: u16,
        graph_data: GraphData,
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
            self.sender.send_single(SenderData::from_vec(data));
        }

        Ok(())
    }

    pub(crate) fn get<T>(&self, idx: u16, getter: impl Fn(&GraphTyped) -> T) -> Option<T> {
        self.graphs.read().get(&idx).map(getter)
    }

    pub(crate) fn count(&self) -> usize {
        self.graphs.read().len()
    }

    pub(crate) fn len(&self, idx: u16) -> Option<usize> {
        self.graphs.read().get(&idx).map(|g| g.y.len())
    }

    pub(crate) fn remove(&self, idx: u16, update: bool) -> Result<(), ()> {
        let mut w = self.graphs.write();
        w.remove(&idx);
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::Graph(self.id, update, GraphHeader::Remove(idx));
            let data = serialize(&header)?;
            self.sender.send(data);
        }
        Ok(())
    }

    pub(crate) fn reset(&self, update: bool) -> Result<(), ()> {
        let mut w = self.graphs.write();
        w.clear();
        if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed) {
            let header = ServerHeader::Graph(self.id, update, GraphHeader::Reset);
            let data = serialize(&header)?;
            self.sender.send(data);
        }
        Ok(())
    }
}

impl EnableTrait for ValueGraphs {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}

impl SyncTrait for ValueGraphs {
    fn sync(&self) -> Result<(), ()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }

        let w = self.graphs.read();

        let header = ServerHeader::Graph(self.id, false, GraphHeader::Reset);
        let data = serialize(&header)?;
        self.sender.send(data);

        for (idx, graph) in w.iter() {
            let data = graph.to_data(self.id, *idx, false, None);
            self.sender.send_single(SenderData::from_vec(data));
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

#[derive(Clone)]
pub(crate) struct GraphTyped {
    pub y: Vec<u8>,
    pub x: Option<Vec<u8>>,
    pub graph_type: GraphType,
}

impl GraphTyped {
    pub fn to_data(
        &self,
        id: u64,
        graph_id: u16,
        update: bool,
        add_points: Option<usize>,
    ) -> Vec<u8> {
        let mut head_buffer = [0u8; 32];

        let mut size = self.y.len();
        let mut data_offset = 0;
        let graph_header = match add_points {
            Some(points) => {
                let info = GraphDataInfo {
                    graph_type: self.graph_type,
                    points: points as u64,
                    is_linear: self.x.is_none(),
                };
                let header = GraphHeader::AddPoints(graph_id, info);
                data_offset = size - points;
                size = points;
                header
            }
            None => {
                let points = self.y.len() / self.graph_type.bytes_size();
                let info = GraphDataInfo {
                    graph_type: self.graph_type,
                    points: points as u64,
                    is_linear: self.x.is_none(),
                };
                GraphHeader::Set(graph_id, info)
            }
        };
        let header = crate::serialization::ServerHeader::Graph(id, update, graph_header);
        let offset = postcard::to_slice(&header, head_buffer[0..].as_mut())
            .expect("Failed to serialize graph data info")
            .len();

        size *= self.graph_type.bytes_size();
        data_offset *= self.graph_type.bytes_size();

        match self.x {
            Some(ref x) => {
                let mut data = vec![0u8; size * 2 + offset];
                data[..offset].copy_from_slice(&head_buffer[..offset]);
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = (x.as_ptr() as *const u8).add(data_offset);
                        std::slice::from_raw_parts(ptr, size)
                    };
                    data[offset..offset + size].copy_from_slice(dat_slice);

                    let dat_slice = unsafe {
                        let ptr = (self.y.as_ptr() as *const u8).add(data_offset);
                        std::slice::from_raw_parts(ptr, size)
                    };
                    data[offset + size..].copy_from_slice(dat_slice);
                }

                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented.");
                }

                data
            }

            None => {
                let mut data = vec![0u8; size + offset];
                data[..offset].copy_from_slice(&head_buffer[..offset]);
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = (self.y.as_ptr() as *const u8).add(data_offset);
                        std::slice::from_raw_parts(ptr, size)
                    };
                    data[offset..].copy_from_slice(dat_slice);
                }

                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented.");
                }

                data
            }
        }
    }
}
