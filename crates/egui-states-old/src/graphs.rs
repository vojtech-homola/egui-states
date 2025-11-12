use parking_lot::RwLock;
use std::sync::Arc;

use serde::Deserialize;

use egui_states_core_old::graphs::{Graph, GraphElement, GraphMessage};
use egui_states_core_old::nohash::NoHashMap;

use crate::UpdateValue;

// pub(crate) trait GraphUpdate: Sync + Send {
//     fn update_graph(&self, data: &[u8]) -> Result<(), String>;
// }

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
        self.graphs.read().get(&idx).map(|g| g.0.clone())
    }

    pub fn len(&self) -> usize {
        self.graphs.read().len()
    }

    pub fn process<R>(&self, idx: u16, op: impl Fn(Option<&Graph<T>>, bool) -> R) -> R {
        let mut g = self.graphs.write();
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

impl<T: GraphElement> UpdateValue for ValueGraphs<T>
where
    T: for<'a> Deserialize<'a>,
{
    fn update_value(&self, data: &[u8]) -> Result<bool, String> {
        let (message, dat) = GraphMessage::deserialize(data).map_err(|e| {
            format!(
                "failed to deserialize graph message: {} with id {}",
                e, self._id
            )
        })?;

        let update = match message {
            GraphMessage::Set(update, idx, info) => {
                let graph = Graph::from_graph_data(info, dat);
                self.graphs.write().insert(idx, (graph, true));
                update
            }
            GraphMessage::AddPoints(update, idx, info) => {
                if let Some((graph, changed)) = self.graphs.write().get_mut(&idx) {
                    graph.add_points_from_data(info, dat)?;
                    *changed = true;
                }
                update
            }
            GraphMessage::Remove(update, idx) => {
                self.graphs.write().remove(&idx);
                update
            }
            GraphMessage::Reset(update) => {
                self.graphs.write().clear();
                update
            }
        };

        Ok(update)
    }
}
