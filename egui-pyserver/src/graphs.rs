use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::buffer::{Element, PyBuffer};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use egui_pytransport::graphs::{GraphMessage, GraphsData, Precision};
use egui_pytransport::transport::WriteMessage;

use crate::SyncTrait;

pub(crate) trait PyGraph: Send + Sync {
    fn all_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn add_points_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn reset_py(&self, update: bool);
}

pub trait GraphType: Element {
    fn precision() -> Precision;
}

impl GraphType for f32 {
    #[inline]
    fn precision() -> Precision {
        Precision::F32
    }
}

impl GraphType for f64 {
    #[inline]
    fn precision() -> Precision {
        Precision::F64
    }
}

struct GraphInner {
    y: Vec<u8>,
    x: Vec<u8>,
}

pub struct ValueGraph<T> {
    id: u32,
    graph: RwLock<GraphInner>,
    precision: Precision,

    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,

    _phantom: std::marker::PhantomData<T>,
}

impl<T: GraphType> ValueGraph<T> {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let graph = RwLock::new(GraphInner {
            y: Vec::new(),
            x: Vec::new(),
        });

        Arc::new(Self {
            id,
            graph,
            precision: T::precision(),
            channel,
            connected,
            _phantom: std::marker::PhantomData,
        })
    }
}

impl<T> PyGraph for ValueGraph<T>
where
    T: Send + Sync + Clone + Element,
{
    fn all_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;

        let shape = buffer.shape();
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

        let line_size = points * size_of::<T>();
        let mut x = vec![0u8; line_size];
        let mut y = vec![0u8; line_size];
        let ptr = buffer.get_ptr(&[0, 0]) as *const u8;
        unsafe { std::ptr::copy_nonoverlapping(ptr, x.as_mut_ptr(), line_size) };
        let ptr = buffer.get_ptr(&[1, 0]) as *const u8;
        unsafe { std::ptr::copy_nonoverlapping(ptr, y.as_mut_ptr(), line_size) };

        let mut w = self.graph.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let mut send_data = vec![0u8; line_size * 2];
            send_data[..line_size].copy_from_slice(&x);
            send_data[line_size..].copy_from_slice(&y);

            let message = GraphMessage::All(GraphsData {
                precision: self.precision,
                points,
                data: send_data,
            });
            self.channel
                .send(WriteMessage::Graph(self.id, update, message))
                .unwrap();
        }

        *w = GraphInner { y, x };

        Ok(())
    }

    fn add_points_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let buffer = PyBuffer::<T>::extract_bound(object)?;
        let shape = buffer.shape();

        if shape.len() != 2 {
            return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
        }
        let points = shape[1];
        if points < 1 {
            return Err(PyValueError::new_err(
                "Added graph data must have at least 1 point.",
            ));
        }

        let mut g = self.graph.write().unwrap();
        let start = g.x.len();
        let ptr = buffer.get_ptr(&[0, 0]) as *const u8;
        let line = unsafe { std::slice::from_raw_parts(ptr, points * size_of::<T>()) };
        g.x.extend_from_slice(line);
        let ptr = buffer.get_ptr(&[1, 0]) as *const u8;
        let line = unsafe { std::slice::from_raw_parts(ptr, points * size_of::<T>()) };
        g.y.extend_from_slice(line);

        if self.connected.load(Ordering::Relaxed) {
            let mut data = vec![0u8; points * size_of::<T>() * 2];
            data[..points * size_of::<T>()].copy_from_slice(&g.x[start..]);
            data[points * size_of::<T>()..].copy_from_slice(&g.y[start..]);

            let message = GraphMessage::AddPoints(GraphsData {
                precision: self.precision,
                points,
                data,
            });
            self.channel
                .send(WriteMessage::Graph(self.id, update, message))
                .unwrap();
        }

        Ok(())
    }

    fn reset_py(&self, update: bool) {
        let mut w = self.graph.write().unwrap();
        w.x.clear();
        w.y.clear();

        if self.connected.load(Ordering::Relaxed) {
            self.channel
                .send(WriteMessage::Graph(self.id, update, GraphMessage::Reset))
                .unwrap();
        }
    }
}

impl<T: Send + Sync> SyncTrait for ValueGraph<T> {
    fn sync(&self) {
        let w = self.graph.read().unwrap();
        if w.x.is_empty() {
            return;
        }

        let mut data = vec![0u8; w.x.len() + w.y.len()];
        data[..w.x.len()].copy_from_slice(&w.x);
        data[w.x.len()..].copy_from_slice(&w.y);

        let message = GraphMessage::All(GraphsData {
            precision: self.precision,
            points: w.x.len() / size_of::<T>(),
            data,
        });
        self.channel
            .send(WriteMessage::Graph(self.id, false, message))
            .unwrap();
    }
}
