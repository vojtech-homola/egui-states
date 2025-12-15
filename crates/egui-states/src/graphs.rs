use parking_lot::RwLock;
use std::sync::Arc;

use serde::Deserialize;

use egui_states_core::graphs::{Graph, GraphElement, GraphHeader};
use egui_states_core::nohash::NoHashMap;

pub(crate) trait UpdateGraph: Sync + Send {
    fn update_graph(&self, header: GraphHeader, data: &[u8]) -> Result<(), String>;
}

pub struct ValueGraphs<T> {
    _id: u64,
    graphs: RwLock<NoHashMap<u16, (Graph<T>, bool)>>,
}

impl<T: Clone + Copy> ValueGraphs<T> {
    pub(crate) fn new(id: u64) -> Arc<Self> {
        Arc::new(Self {
            _id: id,
            graphs: RwLock::new(NoHashMap::default()),
        })
    }

    pub fn get(&self, idx: u16) -> Option<Graph<T>> {
        self.graphs.read().get(&idx).map(|g| g.0.clone())
    }

    pub fn len(&self) -> usize {
        self.graphs.read().len()
    }

    pub fn read<R>(&self, idx: u16, f: impl Fn(Option<&Graph<T>>, bool) -> R) -> R {
        let mut g = self.graphs.write();
        let graph = g.get_mut(&idx);

        match graph {
            Some((graph, changed)) => {
                let r = f(Some(graph), *changed);
                *changed = false;
                r
            }
            None => f(None, false),
        }
    }
}

impl<T: GraphElement> UpdateGraph for ValueGraphs<T>
where
    T: for<'a> Deserialize<'a>,
{
    fn update_graph(&self, header: GraphHeader, data: &[u8]) -> Result<(), String> {
        match header {
            GraphHeader::Set(idx, info) => {
                let graph = Graph::from_graph_data(info, data)?;
                self.graphs.write().insert(idx, (graph, true));
            }
            GraphHeader::AddPoints(idx, info) => {
                if let Some((graph, changed)) = self.graphs.write().get_mut(&idx) {
                    graph.add_points_from_data(info, data)?;
                    *changed = true;
                }
            }
            GraphHeader::Remove(idx) => {
                self.graphs.write().remove(&idx);
            }
            GraphHeader::Reset => {
                self.graphs.write().clear();
            }
        };

        Ok(())
    }
}
