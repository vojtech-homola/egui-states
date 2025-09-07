use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, OnceLock, RwLock, atomic};

use pyo3::buffer::PyBuffer;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyDict, PyList, PyTuple};
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::controls::ControlMessage;
use egui_states_core::nohash::NoHashSet;

use crate::sender::MessageSender;
use crate::server::Server;
use crate::signals::ChangedValues;
use crate::states_server::{PyValuesList, ServerValuesCreator};

// To be able to create all values outside this crate
pub(crate) static CREATE_HOOK: OnceLock<fn(&mut ServerValuesCreator)> = OnceLock::new();

#[pyclass]
pub struct StateServerCore {
    changed_values: ChangedValues,
    values: PyValuesList,

    sender: MessageSender,
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
        // let (channel, rx) = unbounded_channel();
        let connected = Arc::new(atomic::AtomicBool::new(false));
        let (sender, rx) = MessageSender::new();

        let signals = ChangedValues::new();
        let mut values_creator =
            ServerValuesCreator::new(sender.clone(), connected.clone(), signals.clone());

        let creator = CREATE_HOOK.get();
        match creator {
            Some(c) => {
                c(&mut values_creator);
            }
            None => {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "Failed to inicialize state server object.",
                ));
            }
        }

        let (values, py_values, version) = values_creator.get_values();

        let addr = match ip_addr {
            Some(addr) => {
                SocketAddrV4::new(Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]), port)
            }
            None => SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port),
        };
        let server = Server::new(
            sender.clone(),
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
            sender,
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

    #[pyo3(signature=(duration=None))]
    fn update(&self, duration: Option<f32>) {
        if self.connected.load(atomic::Ordering::Relaxed) {
            let message = ControlMessage::Update(duration.unwrap_or(0.0)).serialize();
            self.sender.send(Bytes::from(message));
        }
    }

    // signals ----------------------------------------------------------------
    fn value_set_register(&self, value_id: u32, register: bool) {
        if register {
            self.registed_values.write().unwrap().insert(value_id);
        } else {
            self.registed_values.write().unwrap().remove(&value_id);
        }
    }

    fn value_get_signal<'py>(&self, py: Python<'py>, thread_id: u32) -> (u32, Bound<'py, PyAny>) {
        let (value_id, value) = py.detach(|| {
            loop {
                let res = self.changed_values.wait_changed_value(thread_id);
                if self.registed_values.read().unwrap().contains(&res.0) {
                    break res;
                }
            }
        });
        let arg = value.to_python(py);

        (value_id, arg)
    }

    fn signal_set(&self, value_id: u32, value: &Bound<PyAny>) -> PyResult<()> {
        match self.values.signals.get(&value_id) {
            Some(signal) => signal.set_py(value),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Signal with id {} is not available.",
                value_id
            ))),
        }
    }

    // values -----------------------------------------------------------------
    fn value_set(
        &self,
        value_id: u32,
        value: &Bound<PyAny>,
        set_signal: bool,
        update: bool,
    ) -> PyResult<()> {
        match self.values.values.get(&value_id) {
            Some(setter) => setter.set_py(value, set_signal, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn value_get<'py>(&self, py: Python<'py>, value_id: u32) -> PyResult<Bound<'py, PyAny>> {
        match self.values.values.get(&value_id) {
            Some(getter) => Ok(getter.get_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Value with id {} is not available.",
                value_id
            ))),
        }
    }

    // static values ----------------------------------------------------------
    fn static_set(
        &self,
        py: Python,
        value_id: u32,
        value: Py<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.values.static_values.get(&value_id) {
            Some(static_) => static_.set_py(value.bind(py), update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Static value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn static_get(&self, py: Python, value_id: u32) -> PyResult<Py<PyAny>> {
        match self.values.static_values.get(&value_id) {
            Some(value) => Ok(value.get_py(py).unbind()),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Static value with id {} is not available.",
                value_id
            ))),
        }
    }

    // images -----------------------------------------------------------------
    #[pyo3(signature = (value_id, image, update, origin=None))]
    fn image_set(
        &self,
        py: Python,
        value_id: u32,
        image: PyBuffer<u8>,
        update: bool,
        origin: Option<[usize; 2]>,
    ) -> PyResult<()> {
        match self.values.images.get(&value_id) {
            Some(image_val) => py.detach(|| image_val.set_image_py(&image, origin, update)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Image with id {} is not available.",
                value_id
            ))),
        }
    }

    fn image_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u32,
    ) -> PyResult<(Bound<'py, PyByteArray>, (usize, usize))> {
        match self.values.images.get(&value_id) {
            Some(image) => Ok(image.get_image_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Image with id {} is not available.",
                value_id
            ))),
        }
    }

    fn image_size(&self, value_id: u32) -> PyResult<(usize, usize)> {
        match self.values.images.get(&value_id) {
            Some(image) => Ok(image.get_size_py()),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Image with id {} is not available.",
                value_id
            ))),
        }
    }

    // dicts ------------------------------------------------------------------
    fn dict_get<'py>(&self, py: Python<'py>, value_id: u32) -> PyResult<Bound<'py, PyDict>> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => Ok(dict.get_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn dict_item_get<'py>(
        &self,
        value_id: u32,
        key: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => dict.get_item_py(key),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn dict_set(&self, value_id: u32, dict: &Bound<PyAny>, update: bool) -> PyResult<()> {
        match self.values.dicts.get(&value_id) {
            Some(dict_) => dict_.set_py(dict, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn dict_item_set(
        &self,
        value_id: u32,
        key: &Bound<PyAny>,
        value: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => dict.set_item_py(key, value, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Dict value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn dict_item_del(&self, value_id: u32, key: &Bound<PyAny>, update: bool) -> PyResult<()> {
        match self.values.dicts.get(&value_id) {
            Some(dict) => dict.del_item_py(key, update),
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

    // lists ------------------------------------------------------------------
    fn list_get<'py>(&self, py: Python<'py>, value_id: u32) -> PyResult<Bound<'py, PyList>> {
        match self.values.lists.get(&value_id) {
            Some(list) => Ok(list.get_py(py)),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn list_item_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u32,
        idx: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.get_item_py(py, idx),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn list_set(&self, value_id: u32, list: &Bound<PyAny>, update: bool) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list_) => list_.set_py(list, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn list_item_set(
        &self,
        value_id: u32,
        idx: usize,
        value: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.set_item_py(idx, value, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn list_item_del(&self, value_id: u32, idx: usize, update: bool) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.del_item_py(idx, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "List value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn list_item_add(&self, value_id: u32, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        match self.values.lists.get(&value_id) {
            Some(list) => list.add_item_py(value, update),
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

    // graphs -----------------------------------------------------------------
    #[pyo3(signature = (value_id, idx, graph, update))]
    fn graphs_set(
        &self,
        value_id: u32,
        idx: u16,
        graph: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.values.graphs.get(&value_id) {
            Some(graph_) => graph_.set_py(idx, graph, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn graphs_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u32,
        idx: u16,
    ) -> PyResult<Bound<'py, PyTuple>> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => graph.get_py(py, idx),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    #[pyo3(signature = (value_id, idx, points, update))]
    fn graphs_add_points(
        &self,
        value_id: u32,
        idx: u16,
        points: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => graph.add_points_py(idx, points, update),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn graphs_len(&self, value_id: u32, idx: u16) -> PyResult<usize> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => graph.len_py(idx),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn graphs_remove(&self, value_id: u32, idx: u16, update: bool) -> PyResult<()> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => {
                graph.remove_py(idx, update);
                Ok(())
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn graphs_count(&self, value_id: u32) -> PyResult<u16> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => Ok(graph.count_py()),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn graphs_is_linear(&self, value_id: u32, idx: u16) -> PyResult<bool> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => graph.is_linear_py(idx),
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }

    fn graphs_clear(&self, value_id: u32, update: bool) -> PyResult<()> {
        match self.values.graphs.get(&value_id) {
            Some(graph) => {
                graph.clear_py(update);
                Ok(())
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Graph value with id {} is not available.",
                value_id
            ))),
        }
    }
}
