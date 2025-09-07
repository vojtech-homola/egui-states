use std::mem::size_of;
use std::ptr::copy_nonoverlapping;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use pyo3::buffer::{Element, PyBuffer};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyTuple};
use serde::Serialize;
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::graphs::{Graph, GraphElement, GraphMessage};
use egui_states_core::nohash::NoHashMap;
use egui_states_core::serialization::{TYPE_GRAPH, serialize};

use crate::python_convert::ToPython;
use crate::sender::MessageSender;
use crate::server::SyncTrait;

pub(crate) trait PyGraphTrait: Send + Sync {
    fn set_py(&self, idx: u16, object: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn add_points_py(&self, idx: u16, object: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn get_py<'py>(&self, py: Python<'py>, idx: u16) -> PyResult<Bound<'py, PyTuple>>;
    fn len_py(&self, idx: u16) -> PyResult<usize>;
    fn remove_py(&self, idx: u16, update: bool);
    fn count_py(&self) -> u16;
    fn is_linear_py(&self, idx: u16) -> PyResult<bool>;
    fn clear_py(&self, update: bool);
}

pub(crate) struct PyValueGraphs<T> {
    id: u32,
    graphs: RwLock<NoHashMap<u16, Graph<T>>>,

    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl<T> PyValueGraphs<T> {
    pub(crate) fn new(id: u32, sender: MessageSender, connected: Arc<AtomicBool>) -> Arc<Self> {
        let graphs = RwLock::new(NoHashMap::default());

        Arc::new(Self {
            id,
            graphs,
            sender,
            connected,
        })
    }
}

impl<T> PyGraphTrait for PyValueGraphs<T>
where
    T: GraphElement + Element + for<'py> FromPyObject<'py> + ToPython + Serialize,
{
    fn set_py(&self, idx: u16, object: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;
        let graph = buffer_to_graph(&buffer)?;

        let mut w = self.graphs.write().unwrap();
        if self.connected.load(Ordering::Relaxed) {
            let data = graph.to_data(self.id, idx, update, None);
            self.sender.send(Bytes::from(data));
        }
        w.insert(idx, graph);
        Ok(())
    }

    fn add_points_py(&self, idx: u16, object: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;

        let mut w = self.graphs.write().unwrap();
        let graph = w
            .get_mut(&idx)
            .ok_or_else(|| PyValueError::new_err("Graph not found"))?;
        let points = buffer_to_graph_add(&buffer, graph)?;

        if self.connected.load(Ordering::Relaxed) {
            let data = graph.to_data(self.id, idx, update, Some(points));
            self.sender.send(Bytes::from(data));
        }

        Ok(())
    }

    fn get_py<'py>(&self, py: Python<'py>, idx: u16) -> PyResult<Bound<'py, PyTuple>> {
        let w = self.graphs.read().unwrap();
        let graph = w
            .get(&idx)
            .ok_or_else(|| PyValueError::new_err(format!("Graph with id {} not found", idx)))?;

        match graph.x {
            Some(ref x) => {
                let size = (x.len() + graph.y.len()) * size_of::<T>();
                let bytes = PyByteArray::new_with(py, size, |buf| {
                    let mut ptr = buf.as_mut_ptr() as *mut T;
                    unsafe {
                        std::ptr::copy_nonoverlapping(x.as_ptr(), ptr, x.len());
                        ptr = ptr.add(x.len());
                        std::ptr::copy_nonoverlapping(graph.y.as_ptr(), ptr, graph.y.len());
                    };
                    Ok(())
                })?;

                let shape = (2usize, graph.y.len(), size_of::<T>());
                (bytes, shape).into_pyobject(py)
            }
            None => {
                let size = graph.y.len() * size_of::<T>();
                let data =
                    unsafe { std::slice::from_raw_parts(graph.y.as_ptr() as *const u8, size) };
                let bytes = PyByteArray::new(py, data);
                (bytes, (graph.y.len(), size_of::<T>())).into_pyobject(py)
            }
        }
    }

    fn len_py(&self, idx: u16) -> PyResult<usize> {
        let size = self
            .graphs
            .read()
            .unwrap()
            .get(&idx)
            .ok_or(PyValueError::new_err(format!(
                "Graph with id {} not found",
                idx
            )))?
            .y
            .len();

        Ok(size)
    }

    fn remove_py(&self, idx: u16, update: bool) {
        let mut w = self.graphs.write().unwrap();
        if self.connected.load(Ordering::Relaxed) {
            let message = serialize(self.id, GraphMessage::<T>::Remove(update, idx), TYPE_GRAPH);
            self.sender.send(Bytes::from(message.to_vec()));
        }
        w.remove(&idx);
    }

    fn count_py(&self) -> u16 {
        self.graphs.read().unwrap().len() as u16
    }

    fn is_linear_py(&self, idx: u16) -> PyResult<bool> {
        self.graphs.read().unwrap().get(&idx).map_or(
            Err(PyValueError::new_err(format!(
                "Graph with id {} not found",
                idx
            ))),
            |graph| Ok(graph.x.is_none()),
        )
    }

    fn clear_py(&self, update: bool) {
        let mut w = self.graphs.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let message = serialize(self.id, GraphMessage::<T>::Reset(update), TYPE_GRAPH);
            self.sender.send(Bytes::from(message.to_vec()));
        }
        w.clear();
    }
}

impl<T: GraphElement> SyncTrait for PyValueGraphs<T>
where
    T: Serialize,
{
    fn sync(&self) {
        let w = self.graphs.read().unwrap();

        let message = serialize(self.id, GraphMessage::<T>::Reset(false), TYPE_GRAPH);
        self.sender.send(Bytes::from(message.to_vec()));

        for (idx, graph) in w.iter() {
            let data = graph.to_data(self.id, *idx, false, None);
            self.sender.send(Bytes::from(data));
        }
    }
}

fn buffer_to_graph_add<'py, T>(buffer: &PyBuffer<T>, graph: &mut Graph<T>) -> PyResult<usize>
where
    T: GraphElement + Element + FromPyObject<'py>,
{
    let shape = buffer.shape();
    let stride = buffer.strides().last().ok_or(PyValueError::new_err(
        "Graph data must have at least 1 dimension.",
    ))?;

    if *stride != size_of::<T>() as isize {
        return Err(PyValueError::new_err(
            "Graph line data must have a contiguous memory layout.",
        ));
    }

    if shape.len() == 1 {
        if graph.x.is_some() {
            return Err(PyValueError::new_err(
                "Graph data to add must have the same x axis type.",
            ));
        }

        let points = shape[0];

        let ptr = buffer.get_ptr(&[0]) as *const T;
        let original_len = graph.y.len();
        graph.y.resize(original_len + points, T::zero());
        unsafe { copy_nonoverlapping(ptr, graph.y[original_len..].as_mut_ptr(), points) };

        return Ok(points);
    } else if shape.len() == 2 {
        if graph.x.is_none() {
            return Err(PyValueError::new_err(
                "Graph data to add must have the same x axis type.",
            ));
        }

        let points = shape[1];

        let original_len = graph.x.as_ref().unwrap().len();
        graph
            .x
            .as_mut()
            .unwrap()
            .resize(points + original_len, T::zero());
        let ptr = buffer.get_ptr(&[0, 0]) as *const T;
        unsafe {
            copy_nonoverlapping(
                ptr,
                graph.x.as_mut().unwrap()[original_len..].as_mut_ptr(),
                points,
            )
        };

        let ptr = buffer.get_ptr(&[1, 0]) as *const T;
        let original_len = graph.y.len();
        graph.y.resize(original_len + points, T::zero());
        unsafe { copy_nonoverlapping(ptr, graph.y[original_len..].as_mut_ptr(), points) };

        return Ok(points);
    } else {
        return Err(PyValueError::new_err(
            "Graph data must have 1 or 2 dimensions.",
        ));
    }
}

fn buffer_to_graph<'py, T>(buffer: &PyBuffer<T>) -> PyResult<Graph<T>>
where
    T: GraphElement + Element + FromPyObject<'py>,
{
    let shape = buffer.shape();
    let stride = buffer.strides().last().ok_or(PyValueError::new_err(
        "Graph data must have at least 1 dimension.",
    ))?;

    if *stride != size_of::<T>() as isize {
        return Err(PyValueError::new_err(
            "Graph line data must have a contiguous memory layout.",
        ));
    }

    if shape.len() == 1 {
        if shape[0] < 2 {
            return Err(PyValueError::new_err(
                "Graph data must have at least 2 points.",
            ));
        }

        let points = shape[0];

        let ptr = buffer.get_ptr(&[0]) as *const T;
        let mut y = vec![T::zero(); points];
        unsafe { std::ptr::copy_nonoverlapping(ptr, y.as_mut_ptr(), points) };

        Ok(Graph { y, x: None })
    } else if shape.len() == 2 {
        if shape[0] != 2 {
            return Err(PyValueError::new_err(
                "Graph data must have 2 lines (x, y).",
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

        Ok(Graph { y, x: Some(x) })
    } else {
        return Err(PyValueError::new_err(
            "Graph data must have 1 or 2 dimensions.",
        ));
    }
}
