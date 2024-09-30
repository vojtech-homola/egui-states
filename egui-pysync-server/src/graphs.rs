use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use egui_pysync_common::transport::{self, GraphMessage, Operation, Precision};

use crate::transport::WriteMessage;
use crate::SyncTrait;

pub(crate) trait WriteGraphMessage: Send + Sync {
    fn write_message(&self, head: &mut [u8], stream: &mut TcpStream) -> io::Result<()>;
}

impl WriteGraphMessage for GraphMessage {
    fn write_message(&self, head: &mut [u8], stream: &mut TcpStream) -> io::Result<()> {
        head[6] = match self.precision {
            Precision::F32 => transport::GRAPH_F32,
            Precision::F64 => transport::GRAPH_F64,
        };

        head[7] = match self.operation {
            Operation::Add => transport::GRAPH_ADD,
            Operation::New => transport::GRAPH_NEW,
            Operation::Delete => transport::GRAPH_DELETE,
        };

        head[8..16].copy_from_slice(&(self.count as u64).to_le_bytes());
        head[16..24].copy_from_slice(&(self.lines as u64).to_le_bytes());

        match self.data {
            Some(ref data) => {
                head[24..32].copy_from_slice(&(data.len() as u64).to_le_bytes());
                stream.write_all(head)?;
                stream.write_all(data)
            }
            None => stream.write_all(head),
        }
    }
}

pub(crate) trait PyGraph: Send + Sync {
    fn add_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn new_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn delete_py(&self, update: bool);
}

struct GraphInner {
    data: Vec<Vec<u8>>,
    count: usize,
    precision: Precision,
}

pub struct ValueGraph<T> {
    id: u32,
    graph: RwLock<GraphInner>,

    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,

    _phantom: std::marker::PhantomData<T>,
}

impl<T> ValueGraph<T> {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let graph = RwLock::new(GraphInner {
            data: Vec::new(),
            count: 0,
            precision: Precision::F32,
        });
        Arc::new(Self {
            id,
            graph,
            channel,
            connected,
            _phantom: std::marker::PhantomData,
        })
    }
}

impl<T: Send + Sync> PyGraph for ValueGraph<T> {
    fn new_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()> {
        if object.is_none() {
            let mut w = self.graph.write().unwrap();
            w.data.clear();
            w.count = 0;

            if self.connected.load(Ordering::Relaxed) {
                let message = GraphMessage {
                    precision: Precision::F32,
                    operation: Operation::Delete,
                    count: 0,
                    lines: 0,
                    data: None,
                };
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message))
                    .unwrap();
            }
            return Ok(());
        }

        if let Ok(buffer) = PyBuffer::<f32>::extract_bound(object) {
            let shape = buffer.shape();
            if shape.len() != 2 {
                return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
            }
            let count = shape[1];

            let mut data = Vec::new();
            let line_size = count * size_of::<f32>();
            for i in 0..shape[0] {
                let mut line = vec![0u8; line_size];
                let ptr = buffer.get_ptr(&[i, 0]) as *const u8;
                unsafe { std::ptr::copy_nonoverlapping(ptr, line.as_mut_ptr(), line_size) };
                data.push(line);
            }

            let graph = GraphInner {
                data,
                count,
                precision: Precision::F32,
            };
            let mut w = self.graph.write().unwrap();
            *w = graph;

            if self.connected.load(Ordering::Relaxed) {
                let mut data = vec![0u8; shape[0] * shape[1] * size_of::<f32>()];
                let data_f32 = data.as_mut_ptr() as *mut f32;
                let data_f32 =
                    unsafe { std::slice::from_raw_parts_mut(data_f32, shape[0] * shape[1]) };
                buffer.copy_to_slice(object.py(), data_f32).unwrap();

                let message = GraphMessage {
                    precision: Precision::F32,
                    operation: Operation::New,
                    count: shape[1],
                    lines: shape[0],
                    data: Some(data),
                };
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message))
                    .unwrap();
            }
        } else if let Ok(buffer) = PyBuffer::<f64>::extract_bound(object) {
            let shape = buffer.shape();
            if shape.len() != 2 {
                return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
            }
            let count = shape[1];

            let mut data = Vec::new();
            let line_size = count * std::mem::size_of::<f64>();
            for i in 0..shape[0] {
                let mut line = vec![0u8; line_size];
                let ptr = buffer.get_ptr(&[i, 0]) as *const u8;
                unsafe { std::ptr::copy_nonoverlapping(ptr, line.as_mut_ptr(), line_size) };
                data.push(line);
            }

            let graph = GraphInner {
                data,
                count,
                precision: Precision::F64,
            };
            let mut w = self.graph.write().unwrap();
            *w = graph;

            if self.connected.load(Ordering::Relaxed) {
                let mut data = vec![0u8; shape[0] * shape[1] * size_of::<f64>()];
                let data_f64 = data.as_mut_ptr() as *mut f64;
                let data_f64 =
                    unsafe { std::slice::from_raw_parts_mut(data_f64, shape[0] * shape[1]) };
                buffer.copy_to_slice(object.py(), data_f64).unwrap();

                let message = GraphMessage {
                    precision: Precision::F64,
                    operation: Operation::New,
                    count: shape[1],
                    lines: shape[0],
                    data: Some(data),
                };
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message))
                    .unwrap();
            }
        } else {
            return Err(PyValueError::new_err(
                "Only float32 and float64 are supported for graphs.",
            ));
        }

        Ok(())
    }

    fn add_py(&self, object: &Bound<PyAny>, update: bool) -> PyResult<()> {
        if let Ok(buffer) = PyBuffer::<f32>::extract_bound(object) {
            let shape = buffer.shape();
            if shape.len() != 2 {
                return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
            }
            let count = shape[1];

            let mut w = self.graph.write().unwrap();
            if w.precision != Precision::F32 {
                return Err(PyValueError::new_err("Graph datatype does not match."));
            }
            if w.data.len() != shape[0] {
                return Err(PyValueError::new_err("Graph lines count do not match."));
            }

            let line_size = count * size_of::<f32>();
            for (i, data_line) in w.data.iter_mut().enumerate() {
                let ptr = buffer.get_ptr(&[i, 0]) as *const u8;
                let line = unsafe { std::slice::from_raw_parts(ptr, line_size) };
                data_line.extend_from_slice(&line);
            }

            if self.connected.load(Ordering::Relaxed) {
                let mut data = vec![0u8; shape[0] * shape[1] * size_of::<f32>()];
                let data_f32 = data.as_mut_ptr() as *mut f32;
                let data_f32 =
                    unsafe { std::slice::from_raw_parts_mut(data_f32, shape[0] * shape[1]) };
                buffer.copy_to_slice(object.py(), data_f32).unwrap();

                let message = GraphMessage {
                    precision: Precision::F32,
                    operation: Operation::Add,
                    count: shape[1],
                    lines: shape[0],
                    data: Some(data),
                };
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message))
                    .unwrap();
            }
        } else if let Ok(buffer) = PyBuffer::<f64>::extract_bound(object) {
            let shape = buffer.shape();
            if shape.len() != 2 {
                return Err(PyValueError::new_err("Graph data must have 2 dimensions."));
            }
            let count = shape[1];

            let mut w = self.graph.write().unwrap();
            if w.precision != Precision::F64 {
                return Err(PyValueError::new_err("Graph datatype does not match."));
            }
            if w.data.len() != shape[0] {
                return Err(PyValueError::new_err("Graph lines count do not match."));
            }

            let line_size = count * size_of::<f64>();
            for (i, data_line) in w.data.iter_mut().enumerate() {
                let ptr = buffer.get_ptr(&[i, 0]) as *const u8;
                let line = unsafe { std::slice::from_raw_parts(ptr, line_size) };
                data_line.extend_from_slice(&line);
            }

            if self.connected.load(Ordering::Relaxed) {
                let mut data = vec![0u8; shape[0] * shape[1] * size_of::<f64>()];
                let data_f64 = data.as_mut_ptr() as *mut f64;
                let data_f64 =
                    unsafe { std::slice::from_raw_parts_mut(data_f64, shape[0] * shape[1]) };
                buffer.copy_to_slice(object.py(), data_f64).unwrap();

                let message = GraphMessage {
                    precision: Precision::F64,
                    operation: Operation::Add,
                    count: shape[1],
                    lines: shape[0],
                    data: Some(data),
                };
                self.channel
                    .send(WriteMessage::Graph(self.id, update, message))
                    .unwrap();
            }
        } else {
            return Err(PyValueError::new_err(
                "Only float32 and float64 are supported for graphs.",
            ));
        }

        Ok(())
    }

    fn delete_py(&self, update: bool) {
        let mut w = self.graph.write().unwrap();
        w.data.clear();
        w.count = 0;

        if self.connected.load(Ordering::Relaxed) {
            let message = GraphMessage {
                precision: Precision::F32,
                operation: Operation::Delete,
                count: 0,
                lines: 0,
                data: None,
            };
            self.channel
                .send(WriteMessage::Graph(self.id, update, message))
                .unwrap();
        }
    }
}

impl<T: Send + Sync> SyncTrait for ValueGraph<T> {
    fn sync(&self) {
        let w = self.graph.read().unwrap();
        if w.data.is_empty() {
            return;
        }

        let mut data = Vec::new();
        for line in &w.data {
            data.extend_from_slice(line);
        }

        let message = GraphMessage {
            precision: w.precision,
            operation: Operation::New,
            count: w.count,
            lines: w.data.len(),
            data: Some(data),
        };
        self.channel
            .send(WriteMessage::Graph(self.id, false, message))
            .unwrap();
    }
}
