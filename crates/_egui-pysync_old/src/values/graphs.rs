use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::values_common::{Graph, GraphElement, GraphMessage};
use crate::nohash::NoHashMap;

// pub trait WriteGraphMessage: Send + Sync {
//     fn write_message(self: Box<Self>, head: &mut [u8]) -> Option<Vec<u8>>;
// }

pub(crate) trait GraphUpdate: Sync + Send {
    fn update_graph(&self, data: &[u8]) -> Result<(), String>;
}

pub struct ValueGraphs<T> {
    _id: u32,
    graphs: RwLock<NoHashMap<u16, (Graph<T>, bool)>>,
}

impl<T: Clone + Copy> ValueGraphs<T> {
    pub(crate) fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            _id: id,
            graphs: RwLock::new(NoHashMap::default()),
        })
    }

    pub fn get(&self, idx: u16) -> Option<Graph<T>> {
        self.graphs.read().unwrap().get(&idx).map(|g| g.0.clone())
    }

    pub fn len(&self) -> usize {
        self.graphs.read().unwrap().len()
    }

    pub fn process<R>(&self, idx: u16, op: impl Fn(Option<&Graph<T>>, bool) -> R) -> R {
        let mut g = self.graphs.write().unwrap();
        let graph = g.get_mut(&idx);

        match graph {
            Some((graph, changed)) => {
                let r = op(Some(graph), *changed);
                *changed = false;
                r
            }
            None => op(None, false),
        }
    }
}

impl<T: GraphElement> GraphUpdate for ValueGraphs<T>
where
    T: for<'a> Deserialize<'a>,
{
    fn update_graph(&self, data: &[u8]) -> Result<(), String> {
        let (message, data) = postcard::take_from_bytes(data)
            .map_err(|e| format!("failed to deserialize graph message: {}", e))?;

        match message {
            GraphMessage::Set(idx, info) => {
                let graph = Graph::from_graph_data(info, data);
                self.graphs.write().unwrap().insert(idx, (graph, true));
            }
            GraphMessage::AddPoints(idx, info) => {
                if let Some((graph, changed)) = self.graphs.write().unwrap().get_mut(&idx) {
                    graph.add_points_from_data(info, data)?;
                    *changed = true;
                }
            }
            GraphMessage::Remove(idx) => {
                self.graphs.write().unwrap().remove(&idx);
            }
            GraphMessage::Reset => {
                self.graphs.write().unwrap().clear();
            }
        }

        Ok(())
    }
}
