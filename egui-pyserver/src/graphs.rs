use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::buffer::{Element, PyBuffer};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use egui_pytransport::graphs::{Graph, GraphElement, GraphMessage, XAxis};
use egui_pytransport::transport::WriteMessage;

use crate::SyncTrait;

pub(crate) trait PyGraph: Send + Sync {
    fn add_graph_py(
        &self,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<u16>;
    fn add_points_py(
        &self,
        idx: u16,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<()>;
    fn set_graph_py(
        &self,
        idx: u16,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<()>;
    fn get_graph_py(&self, idx: u16, py: Python) -> PyResult<Bound<PyBytes>>;

    fn len_py(&self) -> u16;
    fn reset_py(&self, update: bool);
}

fn buffer_to_graph<'py, T>(
    py: Python,
    buffer: &PyBuffer<T>,
    range: Option<Bound<'py, PyAny>>,
) -> PyResult<Graph<T>>
where
    T: GraphElement + Element + FromPyObject<'py>,
{
    let shape = buffer.shape();

    let graph = match range {
        Some(range) => {
            let range: [T; 2] = range.extract()?;

            if shape.len() != 1 {
                return Err(PyValueError::new_err(
                    "Graph data with range must have 1 dimension.",
                ));
            }

            if shape[0] < 2 {
                return Err(PyValueError::new_err(
                    "Graph data with range must have at least 2 points.",
                ));
            }

            let points = shape[0];

            let mut y = vec![T::zero(); points];
            buffer.copy_to_slice(py, y.as_mut_slice())?;

            Graph {
                y,
                x: XAxis::Range(range),
            }
        }
        None => {
            if shape.len() != 2 {
                return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
            }

            if shape[0] != 2 {
                return Err(PyValueError::new_err(
                    "Graph data must have at 2 lines (x, y).",
                ));
            }
            if shape[1] < 2 {
                return Err(PyValueError::new_err(
                    "Graph data must have at least 2 points.",
                ));
            }

            let points = shape[1];

            let mut x = vec![T::zero(); points];
            let ptr = buffer.get_ptr(&[0, 0]) as *const T;
            unsafe { std::ptr::copy_nonoverlapping(ptr, x.as_mut_ptr(), points) };

            let mut y = vec![T::zero(); points];
            let ptr = buffer.get_ptr(&[1, 0]) as *const T;
            unsafe { std::ptr::copy_nonoverlapping(ptr, y.as_mut_ptr(), points) };

            Graph { y, x: XAxis::X(x) }
        }
    };

    Ok(graph)
}

pub struct ValueGraph<T> {
    id: u32,
    graphs: RwLock<Vec<Graph<T>>>,

    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl<T> ValueGraph<T> {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let graphs = RwLock::new(Vec::new());

        Arc::new(Self {
            id,
            graphs,
            channel,
            connected,
        })
    }
}

impl<T> PyGraph for ValueGraph<T>
where
    T: GraphElement + Element + for<'py> FromPyObject<'py>,
{
    fn add_graph_py(
        &self,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<u16> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;
        let graph = buffer_to_graph(object.py(), &buffer, range)?;

        let mut w = self.graphs.write().unwrap();
        let idx = w.len() as u16;

        if self.connected.load(Ordering::Relaxed) {
            let graph_data = graph.to_graph_data(None);
            let message = GraphMessage::Add(graph_data);
            self.channel
                .send(WriteMessage::Graph(self.id, update, Box::new(message)))
                .unwrap();
        }
        w.push(graph);

        Ok(idx)
    }

    fn set_graph_py(
        &self,
        idx: u16,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<()> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;
        let graph = buffer_to_graph(object.py(), &buffer, range)?;

        let mut w = self.graphs.write().unwrap();

        if idx as usize >= w.len() {
            return Err(PyValueError::new_err("Index out of range."));
        }

        if self.connected.load(Ordering::Relaxed) {
            let graph_data = graph.to_graph_data(None);
            let message = GraphMessage::Set(idx, graph_data);
            self.channel
                .send(WriteMessage::Graph(self.id, update, Box::new(message)))
                .unwrap();
        }
        w[idx as usize] = graph;

        Ok(())
    }

    fn add_points_py(
        &self,
        idx: u16,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<()> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;
        let shape = buffer.shape();

        let mut w = self.graphs.write().unwrap();

        if idx as usize >= w.len() {
            return Err(PyValueError::new_err("Index out of range."));
        }

        let my_graph = &mut w[idx as usize];

        match (range, &mut my_graph.x) {
            (Some(range), XAxis::Range(my_range)) => {
                let range: [T; 2] = range.extract()?;

                if shape.len() != 1 {
                    return Err(PyValueError::new_err(
                        "Graph data with range must have 1 dimension.",
                    ));
                }

                if shape[0] < 2 {
                    return Err(PyValueError::new_err(
                        "Graph data with range must have at least 2 points.",
                    ));
                }

                let points = shape[0];
                let original_len = my_graph.y.len();
                my_graph.y.resize(points + original_len, T::zero());
                buffer.copy_to_slice(object.py(), &mut my_graph.y[original_len..])?;

                my_range[0] = range[0];
                my_range[1] = range[1];
            }
            (None, XAxis::X(x)) => {
                if shape.len() != 2 {
                    return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
                }

                if shape[0] != 2 {
                    return Err(PyValueError::new_err(
                        "Graph data must have at 2 lines (x, y).",
                    ));
                }
                if shape[1] < 2 {
                    return Err(PyValueError::new_err(
                        "Graph data must have at least 2 points.",
                    ));
                }

                let points = shape[1];
                let original_len = my_graph.y.len();

                x.resize(points + original_len, T::zero());
                let ptr = buffer.get_ptr(&[0, 0]) as *const T;
                unsafe {
                    std::ptr::copy_nonoverlapping(ptr, x[original_len..].as_mut_ptr(), points)
                };

                my_graph.y.resize(points + original_len, T::zero());
                let ptr = buffer.get_ptr(&[1, 0]) as *const T;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        ptr,
                        my_graph.y[original_len..].as_mut_ptr(),
                        points,
                    )
                };
            }

            _ => {
                return Err(PyValueError::new_err(
                    "Mismatch between graph x axis types.",
                ));
            }
        };

        if self.connected.load(Ordering::Relaxed) {
            let message = GraphMessage::AddPoints(idx, my_graph.to_graph_data(None));
            self.channel
                .send(WriteMessage::Graph(self.id, update, Box::new(message)))
                .unwrap();
        }

        Ok(())
    }

    fn get_graph_py(&self, idx: u16, py: Python) -> PyResult<Bound<PyBytes>> {
        unimplemented!("get_graph_py")
    }

    fn len_py(&self) -> u16 {
        self.graphs.read().unwrap().len() as u16
    }

    fn reset_py(&self, update: bool) {
        let mut w = self.graphs.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let message = GraphMessage::<T>::Reset;
            self.channel
                .send(WriteMessage::Graph(self.id, update, Box::new(message)))
                .unwrap();
        }
        w.clear();
    }
}

impl<T: GraphElement> SyncTrait for ValueGraph<T> {
    fn sync(&self) {
        let w = self.graphs.read().unwrap();

        self.channel
            .send(WriteMessage::Graph(
                self.id,
                false,
                Box::new(GraphMessage::<T>::Reset),
            ))
            .unwrap();

        for graph in w.iter() {
            let message = GraphMessage::Add(graph.to_graph_data(None));
            self.channel
                .send(WriteMessage::Graph(self.id, false, Box::new(message)))
                .unwrap();
        }
    }
}
