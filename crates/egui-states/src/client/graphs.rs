use parking_lot::RwLock;
use std::sync::Arc;

use serde::Deserialize;

use crate::graphs::{GraphDataInfo, GraphElement, GraphHeader};
use crate::hashing::NoHashMap;

pub(crate) trait UpdateGraph: Sync + Send {
    fn update_graph(&self, header: GraphHeader, data: &[u8]) -> Result<(), String>;
}

pub struct ValueGraphs<T> {
    name: String,
    graphs: Arc<RwLock<NoHashMap<u16, (Graph<T>, bool)>>>,
}

impl<T: GraphElement + Clone + Copy> ValueGraphs<T> {
    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            graphs: Arc::new(RwLock::new(NoHashMap::default())),
        }
    }

    pub fn get(&self, idx: u16) -> Option<Graph<T>> {
        self.graphs.read().get(&idx).map(|g| g.0.clone())
    }

    pub fn len(&self) -> usize {
        self.graphs.read().len()
    }

    pub fn read<R>(&self, idx: u16, mut f: impl FnMut(Option<&Graph<T>>, bool) -> R) -> R {
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
                let graph = Graph::from_graph_data(info, data)
                    .map_err(|e| format!("Error updating graph {}: {}", self.name, e))?;
                self.graphs.write().insert(idx, (graph, true));
            }
            GraphHeader::AddPoints(idx, info) => {
                if let Some((graph, changed)) = self.graphs.write().get_mut(&idx) {
                    graph
                        .add_points_from_data(info, data)
                        .map_err(|e| format!("Error updating graph {}: {}", self.name, e))?;
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

impl<T> Clone for ValueGraphs<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            graphs: self.graphs.clone(),
        }
    }
}

#[derive(Clone)]
pub struct Graph<T> {
    pub y: Vec<T>,
    pub x: Option<Vec<T>>,
}

impl<T: GraphElement> Graph<T> {
    pub(crate) fn add_points_from_data(
        &mut self,
        info: GraphDataInfo,
        data: &[u8],
    ) -> Result<(), String> {
        let GraphDataInfo {
            graph_type,
            points,
            is_linear,
        } = info;
        let points = points as usize;

        if graph_type != T::graph_type() {
            return Err("Incoming Graph data type does not match.".to_string());
        }

        #[cfg(target_endian = "little")]
        {
            // TODO: Do some checks to make sure the incoming data is compatible with
            // the existing graph
            match (&mut self.x, is_linear) {
                (Some(x), false) => {
                    let old_size = x.len();
                    x.resize(old_size + points, T::default());
                    let mut ptr = data.as_ptr() as *const T;
                    let data_slice = unsafe { std::slice::from_raw_parts(ptr, points) };
                    x[old_size..].copy_from_slice(data_slice);

                    self.y.resize(old_size + points, T::default());
                    let data_slice = unsafe {
                        ptr = ptr.add(points);
                        std::slice::from_raw_parts(ptr, points)
                    };
                    self.y[old_size..].copy_from_slice(data_slice);

                    Ok(())
                }
                (None, true) => {
                    let old_size = self.y.len();
                    self.y.resize(old_size + points, T::default());
                    let data_slice = unsafe {
                        let ptr = data.as_ptr() as *const T;
                        std::slice::from_raw_parts(ptr, points)
                    };
                    self.y[old_size..].copy_from_slice(data_slice);

                    Ok(())
                }
                _ => return Err("Incoming Graph data and graph are not compatible.".to_string()),
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented.");
        }
    }

    pub(crate) fn from_graph_data(info: GraphDataInfo, data: &[u8]) -> Result<Self, String> {
        let GraphDataInfo {
            graph_type,
            is_linear,
            points,
        } = info;
        let points = points as usize;

        if T::graph_type() != graph_type {
            return Err("Incoming Graph data type does not match.".to_string());
        }

        #[cfg(target_endian = "little")]
        {
            match is_linear {
                true => {
                    let mut y: Vec<T> = Vec::with_capacity(points);
                    let y_ptr = y.as_mut_ptr() as *mut u8;
                    let bytes = points * size_of::<T>();
                    unsafe {
                        std::ptr::copy_nonoverlapping(data.as_ptr(), y_ptr, bytes);
                        y.set_len(points);
                    }

                    Ok(Graph { x: None, y })
                }
                false => {
                    let bytes = points * size_of::<T>();
                    let mut x: Vec<T> = Vec::with_capacity(points);
                    let ptr = x.as_mut_ptr() as *mut u8;
                    let mut data_ptr = data.as_ptr();
                    unsafe {
                        std::ptr::copy_nonoverlapping(data_ptr, ptr, bytes);
                        x.set_len(points);
                    }
                    let mut y: Vec<T> = Vec::with_capacity(points);
                    let ptr = y.as_mut_ptr() as *mut u8;
                    unsafe {
                        data_ptr = data_ptr.add(bytes);
                        std::ptr::copy_nonoverlapping(data_ptr, ptr, bytes);
                        y.set_len(points);
                    }

                    Ok(Graph { x: Some(x), y })
                }
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented.");
        }
    }
}
