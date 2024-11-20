use std::sync::{Arc, RwLock};

use egui_pysync::graphs::{GraphElement, GraphMessage};
use egui_pysync::nohash::NoHashMap;

pub use egui_pysync::graphs::Graph;

pub(crate) trait GraphUpdate: Sync + Send {
    fn update_graph(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String>;
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

impl<T: GraphElement> GraphUpdate for ValueGraphs<T> {
    fn update_graph(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String> {
        let message: GraphMessage<T> = GraphMessage::read_message(head, data)?;

        match message {
            GraphMessage::Set(idx, graph_data) => {
                let graph = Graph::from_graph_data(graph_data);
                self.graphs.write().unwrap().insert(idx, (graph, true));
            }
            GraphMessage::AddPoints(idx, graph_data) => {
                if let Some((graph, changed)) = self.graphs.write().unwrap().get_mut(&idx) {
                    graph.add_points_from_data(graph_data)?;
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
