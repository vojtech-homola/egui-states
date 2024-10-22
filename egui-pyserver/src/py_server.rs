use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{
    atomic,
    mpsc::{self, Sender},
    Arc, OnceLock, RwLock,
};

use pyo3::buffer::PyBuffer;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use egui_pytransport::commands::CommandMessage;
use egui_pytransport::transport::WriteMessage;
use egui_pytransport::NoHashSet;

use crate::server::Server;
use crate::signals::ChangedValues;
use crate::states_creator::{PyValuesList, ValuesCreator};

// To be able to create all values outside this crate
pub(crate) static CREATE_HOOK: OnceLock<fn(&mut ValuesCreator)> = OnceLock::new();

#[pyclass]
pub(crate) struct StateServerCore {
    changed_values: ChangedValues,
    values: PyValuesList,

    channel: Sender<WriteMessage>,
    connected: Arc<atomic::AtomicBool>,
    server: RwLock<Server>,
    registed_values: RwLock<NoHashSet<u32>>,
}

impl Drop for StateServerCore {
    fn drop(&mut self) {
        self.server.write().unwrap().stop();
    }
}

#[pymethods]
impl StateServerCore {
    #[new]
    #[pyo3(signature = (port, ip_addr=None, handshake=None))]
    fn new(port: u16, ip_addr: Option<[u8; 4]>, handshake: Option<Vec<u64>>) -> PyResult<Self> {
        let (channel, rx) = mpsc::channel();
        let connected = Arc::new(atomic::AtomicBool::new(false));

        let signals = ChangedValues::new();
        let mut values_creator =
            ValuesCreator::new(channel.clone(), connected.clone(), signals.clone());

        let creator = CREATE_HOOK.get();
        match creator {
            Some(c) => {
                let _ = c(&mut values_creator);
            }
            None => {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "Failed to inicialize state server object.",
                ))
            }
        }

        let (values, py_values, version) = values_creator.get_values();

        let addr = match ip_addr {
            Some(addr) => {
                SocketAddrV4::new(Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]), port)
            }
            None => SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port),
        };
        let server = Server::new(
            channel.clone(),
            rx,
            connected.clone(),
            values,
            signals.clone(),
            addr,
            version,
            handshake,
        );

        let obj = Self {
            changed_values: signals,
            values: py_values,
            channel,
            connected,
            server: RwLock::new(server),
            registed_values: RwLock::new(NoHashSet::default()),
        };

        Ok(obj)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(atomic::Ordering::Relaxed)
    }

    fn is_running(&self) -> bool {
        self.server.read().unwrap().is_running()
    }

    fn start(&self) {
        self.server.write().unwrap().start();
    }

    fn stop(&self) {
        self.server.write().unwrap().stop();
    }

    fn disconnect_client(&self) {
        self.server.write().unwrap().disconnect_client();
    }

    fn set_register_value(&self, value_id: u32, register: bool) {
        if register {
            self.registed_values.write().unwrap().insert(value_id);
        } else {
            self.registed_values.write().unwrap().remove(&value_id);
        }
    }

    fn get_signal_value<'py, 'a>(
        &'a self,
        py: Python<'py>,
        thread_id: u32,
    ) -> (u32, Bound<'py, PyTuple>) {
        let (value_id, value) = py.allow_threads(|| loop {
            let res = self.changed_values.wait_changed_value(thread_id);
            if self.registed_values.read().unwrap().contains(&res.0) {
                break res;
            }
        });
        let args = PyTuple::new_bound(py, [value.to_object(py)]);

        (value_id, args)
    }

    fn set_value(
        &self,
        py: Python,
        value_id: u32,
        value: PyObject,
        set_signal: bool,
        update: bool,
    ) -> PyResult<()> {
        match self.values.values.get(&value_id) {
            Some(setter) => setter.set_py(value.bind(py), set_signal, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn get_value(&self, py: Python, value_id: u32) -> PyResult<PyObject> {
        match self.values.values.get(&value_id) {
            Some(getter) => Ok(getter.get_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn set_static(&self, py: Python, value_id: u32, value: PyObject, update: bool) -> PyResult<()> {
        match self.values.static_values.get(&value_id) {
            Some(static_) => static_.set_py(value.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Static value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn get_static(&self, py: Python, value_id: u32) -> PyResult<PyObject> {
        match self.values.static_values.get(&value_id) {
            Some(value) => Ok(value.get_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Static value with id {} is not available.",
                value_id
            ))),
        }
    }

    #[pyo3(signature = (value_id, image, update, rect=None))]
    fn set_image(
        &self,
        py: Python,
        value_id: u32,
        image: PyBuffer<u8>,
        update: bool,
        rect: Option<[usize; 4]>,
    ) -> PyResult<()> {
        match self.values.images.get(&value_id) {
            Some(image_val) => {
                let image_data = image.to_vec(py)?;
                let shape = image.shape().to_vec();
                py.allow_threads(|| image_val.set_image_py(image_data, shape, rect, update))
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Image with id {} is not available.",
                value_id
            ))),
        }
    }

    #[pyo3(signature = (value_id, update, histogram=None))]
    fn set_histogram(
        &self,
        py: Python,
        value_id: u32,
        update: bool,
        histogram: Option<PyBuffer<f32>>,
    ) -> PyResult<()> {
        match self.values.images.get(&value_id) {
            Some(image_val) => {
                let histogram = match histogram {
                    Some(hist) => Some(hist.to_vec(py)?),
                    None => None,
                };

                py.allow_threads(|| image_val.set_histogram_py(histogram, update))
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Image with id {} is not available.",
                value_id
            ))),
        }
    }

    fn get_dict(&self, py: Python, value_id: u32) -> PyResult<PyObject> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => Ok(dict.get_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn get_dict_item(&self, py: Python, value_id: u32, key: PyObject) -> PyResult<PyObject> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => dict.get_item_py(key.bind(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn set_dict(&self, py: Python, value_id: u32, dict: PyObject, update: bool) -> PyResult<()> {
        match self.values.dicts.get(&value_id) {
            Some(dict_) => dict_.set_py(dict.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn set_dict_item(
        &self,
        py: Python,
        value_id: u32,
        key: PyObject,
        value: PyObject,
        update: bool,
    ) -> PyResult<()> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => dict.set_item_py(key.bind(py), value.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn del_dict_item(
        &self,
        py: Python,
        value_id: u32,
        key: PyObject,
        update: bool,
    ) -> PyResult<()> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => dict.del_item_py(key.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn dict_len(&self, value_id: u32) -> PyResult<usize> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => Ok(dict.len_py()),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn get_list(&self, py: Python, value_id: u32) -> PyResult<PyObject> {
        match self.values.lists.get(&value_id) {
            Some(list) => Ok(list.get_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn get_list_item(&self, py: Python, value_id: u32, idx: usize) -> PyResult<PyObject> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.get_item_py(py, idx),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn set_list(&self, py: Python, value_id: u32, list: PyObject, update: bool) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list_) => list_.set_py(list.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn set_list_item(
        &self,
        py: Python,
        value_id: u32,
        idx: usize,
        value: PyObject,
        update: bool,
    ) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.set_item_py(idx, value.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn del_list_item(&self, value_id: u32, idx: usize, update: bool) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.del_item_py(idx, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn add_list_item(
        &self,
        py: Python,
        value_id: u32,
        value: PyObject,
        update: bool,
    ) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.add_item_py(value.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn list_len(&self, value_id: u32) -> PyResult<usize> {
        match self.values.lists.get(&value_id) {
            Some(list) => Ok(list.len_py()),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn set_graph(&self, py: Python, value_id: u32, graph: PyObject, update: bool) -> PyResult<()> {
        match self.values.graphs.get(&value_id) {
            Some(graph_) => graph_.all_py(graph.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn add_graph_points(
        &self,
        py: Python,
        value_id: u32,
        node: PyObject,
        update: bool,
    ) -> PyResult<()> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => graph.add_points_py(node.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn clear_graph(&self, value_id: u32, update: bool) -> PyResult<()> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => {
                graph.reset_py(update);
                Ok(())
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    #[pyo3(signature=(duration=None))]
    fn update(&self, duration: Option<f32>) {
        if self.connected.load(atomic::Ordering::Relaxed) {
            let message = CommandMessage::Update(duration.unwrap_or(0.0));
            self.channel.send(WriteMessage::Command(message)).unwrap();
        }
    }
}
