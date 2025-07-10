use std::mem::size_of;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::nohash::NoHashMap;

pub trait WriteGraphMessage: Send + Sync {
    fn write_message(self: Box<Self>, head: &mut [u8]) -> Option<Vec<u8>>;
}
pub trait GraphElement: Clone + Copy + Send + Sync + 'static {
    fn zero() -> Self;
}

#[derive(Clone)]
pub struct Graph<T> {
    pub y: Vec<T>,
    pub x: Option<Vec<T>>,
}

impl<T: GraphElement> Graph<T> {
    #[cfg(feature = "server")]
    fn to_graph_data(&self) -> (GraphDataInfo<T>, Vec<u8>) {
        let bytes_size = std::mem::size_of::<T>() * self.y.len();
        let points = self.y.len();

        match self.x {
            Some(ref x) => {
                let mut data = vec![0u8; bytes_size * 2];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = x.as_ptr() as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data[..bytes_size].copy_from_slice(dat_slice);

                    let dat_slice = unsafe {
                        let ptr = self.y.as_ptr() as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data[bytes_size..].copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                (GraphDataInfo::new(points, false), data)
            }

            None => {
                let mut data = vec![0u8; bytes_size];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = self.y.as_ptr() as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data.copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                (GraphDataInfo::new(points, true), data)
            }
        }
    }

    fn add_points_from_data(&mut self, info: GraphDataInfo<T>, data: &[u8]) -> Result<(), String> {
        let GraphDataInfo {
            points, is_linear, ..
        } = info;

        #[cfg(target_endian = "little")]
        {
            match (&mut self.x, is_linear) {
                (Some(x), false) => {
                    let old_size = x.len();
                    x.resize(old_size + points, T::zero());
                    let mut ptr = data.as_ptr() as *const T;
                    let data_slice = unsafe { std::slice::from_raw_parts(ptr, points) };
                    x[old_size..].copy_from_slice(data_slice);

                    self.y.resize(old_size + points, T::zero());
                    let data_slice = unsafe {
                        ptr = ptr.add(points);
                        std::slice::from_raw_parts(ptr, points)
                    };
                    self.y[old_size..].copy_from_slice(data_slice);

                    Ok(())
                }
                (None, true) => {
                    let old_size = self.y.len();
                    self.y.resize(old_size + points, T::zero());
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
            unimplemented!("Big endian not implemented yet.");
        }
    }

    fn from_graph_data(info: GraphDataInfo<T>, data: &[u8]) -> Self {
        let GraphDataInfo {
            is_linear, points, ..
        } = info;

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

                    Graph { x: None, y }
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

                    Graph { x: Some(x), y }
                }
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented yet.");
        }
    }
}

#[derive(Serialize, Deserialize)]
struct GraphDataInfo<T> {
    phantom: std::marker::PhantomData<T>,
    is_linear: bool,
    points: usize,
}

#[cfg(feature = "server")]
impl<T> GraphDataInfo<T> {
    fn new(points: usize, is_linear: bool) -> Self {
        Self {
            phantom: std::marker::PhantomData,
            is_linear,
            points,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum GraphMessage<T> {
    Set(u16, GraphDataInfo<T>),
    AddPoints(u16, GraphDataInfo<T>),
    Remove(u16),
    Reset,
}

// CLIENT --------------------------------------------------------------------
// ---------------------------------------------------------------------------
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

// SERVER --------------------------------------------------------------------
// ---------------------------------------------------------------------------
#[cfg(feature = "server")]
pub(crate) mod server {
    use super::*;

    use std::ptr::copy_nonoverlapping;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;

    use pyo3::buffer::{Element, PyBuffer};
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;
    use pyo3::types::{PyByteArray, PyTuple};

    use crate::python_convert::ToPython;
    use crate::server::SyncTrait;
    use crate::transport::{serialize, WriteMessage};

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

        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    }

    impl<T> PyValueGraphs<T> {
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

    impl<T> PyGraphTrait for PyValueGraphs<T>
    where
        T: GraphElement + Element + for<'py> FromPyObject<'py> + ToPython + Serialize,
    {
        fn set_py(&self, idx: u16, object: &Bound<PyAny>, update: bool) -> PyResult<()> {
            let buffer = PyBuffer::<T>::extract_bound(object)?;
            let graph = buffer_to_graph(&buffer)?;

            let mut w = self.graphs.write().unwrap();
            if self.connected.load(Ordering::Relaxed) {
                let (info, data) = graph.to_graph_data();
                let message = serialize(GraphMessage::Set(idx, info));
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message, Some(data)))
                    .unwrap();
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
            buffer_to_graph_add(&buffer, graph)?;

            if self.connected.load(Ordering::Relaxed) {
                let (info, data) = graph.to_graph_data();
                let message = serialize(GraphMessage::AddPoints(idx, info));
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message, Some(data)))
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
                let message = serialize(GraphMessage::<T>::Remove(idx));
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message, None))
                    .unwrap();
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
                let message = serialize(GraphMessage::<T>::Reset);
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message, None))
                    .unwrap();
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

            let message = serialize(GraphMessage::<T>::Reset);
            self.channel
                .send(WriteMessage::Graph(self.id, false, message, None))
                .unwrap();

            for (idx, graph) in w.iter() {
                let (info, data) = graph.to_graph_data();
                let message = serialize(GraphMessage::Set(*idx, info));
                self.channel
                    .send(WriteMessage::Graph(self.id, false, message, Some(data)))
                    .unwrap();
            }
        }
    }

    fn buffer_to_graph_add<'py, T>(buffer: &PyBuffer<T>, graph: &mut Graph<T>) -> PyResult<()>
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
        } else {
            return Err(PyValueError::new_err(
                "Graph data must have 1 or 2 dimensions.",
            ));
        }

        Ok(())
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
}

// GraphElement --------------------------------------------------------------
// ---------------------------------------------------------------------------
impl GraphElement for f32 {
    #[inline]
    fn zero() -> Self {
        0.0
    }
}

impl GraphElement for f64 {
    #[inline]
    fn zero() -> Self {
        0.0
    }
}
