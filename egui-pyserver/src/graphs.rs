use std::ptr::copy_nonoverlapping;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::buffer::{Element, PyBuffer};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};

use egui_pysync::graphs::{Graph, GraphElement, GraphMessage, XAxis};
use egui_pysync::nohash::NoHashMap;
use egui_pysync::transport::WriteMessage;

use crate::{SyncTrait, ToPython};

pub(crate) trait PyGraph: Send + Sync {
    fn set_py(
        &self,
        idx: u16,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<()>;
    fn add_points_py(
        &self,
        idx: u16,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<()>;
    fn get_py<'py>(&self, py: Python<'py>, idx: u16) -> PyResult<Bound<'py, PyTuple>>;
    fn len_py(&self, idx: u16) -> PyResult<usize>;
    fn remove_py(&self, idx: u16, update: bool);
    fn count_py(&self) -> u16;
    fn clear_py(&self, update: bool);
}

pub struct ValueGraphs<T> {
    id: u32,
    graphs: RwLock<NoHashMap<u16, Graph<T>>>,

    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl<T> ValueGraphs<T> {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let graphs = RwLock::new(NoHashMap::default());

        Arc::new(Self {
            id,
            graphs,
            channel,
            connected,
        })
    }
}

impl<T> PyGraph for ValueGraphs<T>
where
    T: GraphElement + Element + for<'py> FromPyObject<'py> + ToPython,
{
    fn set_py(
        &self,
        idx: u16,
        object: &Bound<PyAny>,
        range: Option<Bound<PyAny>>,
        update: bool,
    ) -> PyResult<()> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;
        let range = match range {
            Some(range) => Some(range.extract::<[T; 2]>()?),
            None => None,
        };
        let graph = buffer_to_graph(&buffer, range)?;

        let mut w = self.graphs.write().unwrap();
        if self.connected.load(Ordering::Relaxed) {
            let graph_data = graph.to_graph_data(None);
            let message = GraphMessage::Set(idx, graph_data);
            self.channel
                .send(WriteMessage::Graph(self.id, update, Box::new(message)))
                .unwrap();
        }
        w.insert(idx, graph);
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
        let range = match range {
            Some(range) => Some(range.extract::<[T; 2]>()?),
            None => None,
        };

        let mut w = self.graphs.write().unwrap();
        let graph = w
            .get_mut(&idx)
            .ok_or_else(|| PyValueError::new_err("Graph not found"))?;
        buffer_to_graph_add(&buffer, range, graph)?;

        if self.connected.load(Ordering::Relaxed) {
            let message = GraphMessage::AddPoints(idx, graph.to_graph_data(None));
            self.channel
                .send(WriteMessage::Graph(self.id, update, Box::new(message)))
                .unwrap();
        }

        Ok(())
    }

    fn get_py<'py>(&self, py: Python<'py>, idx: u16) -> PyResult<Bound<'py, PyTuple>> {
        let w = self.graphs.read().unwrap();
        let graph = w
            .get(&idx)
            .ok_or_else(|| PyValueError::new_err(format!("Graph with id {} not found", idx)))?;

        match graph.x {
            XAxis::X(ref x) => {
                let size = (x.len() + graph.y.len()) * size_of::<T>();
                let bytes = PyBytes::new_with(py, size, |buf| {
                    let mut ptr = buf.as_mut_ptr() as *mut T;
                    unsafe {
                        std::ptr::copy_nonoverlapping(x.as_ptr(), ptr, x.len());
                        ptr = ptr.add(x.len());
                        std::ptr::copy_nonoverlapping(graph.y.as_ptr(), ptr, graph.y.len());
                    };
                    Ok(())
                })?;

                let shape = (2usize, graph.y.len(), size_of::<T>());
                (bytes, shape, None::<Bound<PyTuple>>).into_pyobject(py)
            }
            XAxis::Range(range) => {
                let size = graph.y.len() * size_of::<T>();
                let data =
                    unsafe { std::slice::from_raw_parts(graph.y.as_ptr() as *const u8, size) };
                let bytes = PyBytes::new(py, data);
                let range = PyTuple::new(py, [range[0].to_python(py), range[1].to_python(py)])?;
                (bytes, (graph.y.len(), size_of::<T>()), Some(range)).into_pyobject(py)
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
            let message = GraphMessage::<T>::Remove(idx);
            self.channel
                .send(WriteMessage::Graph(self.id, update, Box::new(message)))
                .unwrap();
        }
        w.remove(&idx);
    }

    fn count_py(&self) -> u16 {
        self.graphs.read().unwrap().len() as u16
    }

    fn clear_py(&self, update: bool) {
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

impl<T: GraphElement> SyncTrait for ValueGraphs<T> {
    fn sync(&self) {
        let w = self.graphs.read().unwrap();

        self.channel
            .send(WriteMessage::Graph(
                self.id,
                false,
                Box::new(GraphMessage::<T>::Reset),
            ))
            .unwrap();

        for (idx, graph) in w.iter() {
            let message = GraphMessage::Set(*idx, graph.to_graph_data(None));
            self.channel
                .send(WriteMessage::Graph(self.id, false, Box::new(message)))
                .unwrap();
        }
    }
}

fn buffer_to_graph_add<'py, T>(
    buffer: &PyBuffer<T>,
    range: Option<[T; 2]>,
    graph: &mut Graph<T>,
) -> PyResult<()>
where
    T: GraphElement + Element + FromPyObject<'py>,
{
    let shape = buffer.shape();
    match range {
        Some(range) => {
            if shape.len() != 1 {
                return Err(PyValueError::new_err(
                    "Graph data with range must have 1 dimension.",
                ));
            }

            match graph.x {
                XAxis::Range(ref mut r) => {
                    let points = shape[0];
                    let ptr = buffer.get_ptr(&[0]) as *const T;
                    let original_len = graph.y.len();
                    graph.y.resize(original_len + points, T::zero());
                    unsafe {
                        copy_nonoverlapping(ptr, graph.y[original_len..].as_mut_ptr(), points)
                    };
                    *r = range;
                }
                XAxis::X(_) => {
                    return Err(PyValueError::new_err(
                        "Graph data with range must have the same x axis type.",
                    ));
                }
            }
            Ok(())
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

            match graph.x {
                XAxis::X(ref mut x) => {
                    let points = shape[1];
                    let original_len = x.len();
                    x.resize(points + original_len, T::zero());
                    let ptr = buffer.get_ptr(&[0, 0]) as *const T;
                    unsafe { copy_nonoverlapping(ptr, x[original_len..].as_mut_ptr(), points) };
                }
                XAxis::Range(_) => {
                    return Err(PyValueError::new_err(
                        "Graph data with range must have the same x axis type.",
                    ));
                }
            }
            Ok(())
        }
    }
}

fn buffer_to_graph<'py, T>(buffer: &PyBuffer<T>, range: Option<[T; 2]>) -> PyResult<Graph<T>>
where
    T: GraphElement + Element + FromPyObject<'py>,
{
    let shape = buffer.shape();
    match range {
        Some(range) => {
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

            let ptr = buffer.get_ptr(&[0]) as *const T;
            let mut y = vec![T::zero(); points];
            unsafe { std::ptr::copy_nonoverlapping(ptr, y.as_mut_ptr(), points) };

            let x = XAxis::Range(range);
            Ok(Graph { y, x })
        }
        None => {
            if shape.len() != 2 {
                return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
            }
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

            Ok(Graph { y, x: XAxis::X(x) })
        }
    }
}
