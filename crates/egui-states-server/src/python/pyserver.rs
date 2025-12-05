use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::sync::OnceLock;

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyDict, PyList, PyTuple};

use egui_states_core::generate_value_id;
use egui_states_core::graphs::GraphType;
use egui_states_core::nohash::NoHashMap;

use crate::python::{
    pygraphs, pyimage, pyparsing,
    pytypes::{ObjectType, PyObjectType},
};
use crate::server::{Server, StatesList};
use crate::signals::SignalsManager;
use crate::value_parsing::{ValueCreator, ValueParser};
use crate::values::{Value, ValueStatic};
use crate::{graphs::ValueGraphs, image::ValueImage, list::ValueList, map::ValueMap};

struct CoreInner {
    states: StatesList,
    types: NoHashMap<u64, ObjectType>,
}

#[pyclass]
pub struct StateServerCore {
    server: RwLock<Server>,
    signals: SignalsManager,
    inner: OnceLock<CoreInner>,
    temps: RwLock<Option<NoHashMap<u64, ObjectType>>>,
}

impl StateServerCore {
    #[inline]
    fn get_inner(&self) -> PyResult<&CoreInner> {
        match self.inner.get() {
            Some(inner) => Ok(inner),
            None => Err(PyValueError::new_err("Server not initialized.")),
        }
    }

    #[inline]
    fn inner_values(&self, value_id: u64) -> PyResult<(&Arc<Value>, &ObjectType)> {
        let inner = self.get_inner()?;
        match (
            inner.states.values.get(&value_id),
            inner.types.get(&value_id),
        ) {
            (Some(value), Some(object_type)) => Ok((value, object_type)),
            _ => Err(PyValueError::new_err("Value ID not found.")),
        }
    }

    #[inline]
    fn inner_static(&self, value_id: u64) -> PyResult<(&Arc<ValueStatic>, &ObjectType)> {
        let inner = self.get_inner()?;
        match (
            inner.states.static_values.get(&value_id),
            inner.types.get(&value_id),
        ) {
            (Some(value), Some(object_type)) => Ok((value, object_type)),
            _ => Err(PyValueError::new_err("Value ID not found.")),
        }
    }

    #[inline]
    fn inner_list(&self, value_id: u64) -> PyResult<(&Arc<ValueList>, &ObjectType)> {
        let inner = self.get_inner()?;
        match (
            inner.states.lists.get(&value_id),
            inner.types.get(&value_id),
        ) {
            (Some(list), Some(value_type)) => Ok((list, value_type)),
            _ => Err(PyValueError::new_err("Value ID not found.")),
        }
    }

    #[inline]
    fn inner_map(&self, value_id: u64) -> PyResult<(&Arc<ValueMap>, &ObjectType, &ObjectType)> {
        let inner = self.get_inner()?;
        match (inner.states.maps.get(&value_id), inner.types.get(&value_id)) {
            (Some(map), Some(ObjectType::Map(key_type, value_type))) => {
                Ok((map, key_type, value_type))
            }
            _ => Err(PyValueError::new_err(
                "Value ID not found or type mismatch.",
            )),
        }
    }

    #[inline]
    fn inner_image(&self, value_id: u64) -> PyResult<&Arc<ValueImage>> {
        let inner = self.get_inner()?;
        match inner.states.images.get(&value_id) {
            Some(image) => Ok(image),
            _ => Err(PyValueError::new_err("Value ID not found.")),
        }
    }

    #[inline]
    fn inner_graphs(&self, value_id: u64) -> PyResult<&Arc<ValueGraphs>> {
        let inner = self.get_inner()?;
        match inner.states.graphs.get(&value_id) {
            Some(graphs) => Ok(graphs),
            _ => Err(PyValueError::new_err("Value ID not found.")),
        }
    }
}

#[pymethods]
impl StateServerCore {
    #[new]
    #[pyo3(signature = (port, ip_addr=None, handshake=None))]
    fn new(port: u16, ip_addr: Option<[u8; 4]>, handshake: Option<Vec<u64>>) -> PyResult<Self> {
        let addr = match ip_addr {
            Some(addr) => {
                SocketAddrV4::new(Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]), port)
            }
            None => SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port),
        };

        let server = Server::new(addr, handshake);
        let signals = server.get_signals_manager();

        // register logging signal type
        let logging_object_type = ObjectType::Tuple(vec![ObjectType::U8, ObjectType::String]);
        let mut types = NoHashMap::default();
        types.insert(signals.get_logging_id(), logging_object_type);

        Ok(Self {
            server: RwLock::new(server),
            signals,
            inner: OnceLock::new(),
            temps: RwLock::new(Some(types)),
        })
    }

    fn initialize(&self) {
        let states = self.server.write().initialize();
        if let Some(states) = states {
            if let Some(types) = self.temps.write().take() {
                let _ = self.inner.set(CoreInner { states, types });
            }
        }
    }

    fn start(&self) {
        self.server.write().start();
    }

    fn stop(&self) {
        self.server.write().stop();
    }

    fn is_running(&self) -> bool {
        self.server.read().is_running()
    }

    fn is_connected(&self) -> bool {
        self.server.read().is_connected()
    }

    fn disconnect_clients(&self) {
        self.server.write().disconnect_client();
    }

    fn update(&self, duration: Option<f32>) {
        self.server.write().update(duration);
    }

    // values -----------------------------------------------------------
    fn value_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyAny>> {
        let (value, object_type) = self.inner_values(value_id)?;
        let mut parser = ValueParser::new(value.get());
        pyparsing::deserialize_py(py, &mut parser, object_type)
    }

    fn value_set(
        &self,
        value_id: u64,
        value: &Bound<PyAny>,
        set_signal: bool,
        update: bool,
    ) -> PyResult<()> {
        let (val, object_type) = self.inner_values(value_id)?;
        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(value, object_type, &mut creator)?;
        let data = creator.finalize();
        val.set(data, set_signal, update);
        Ok(())
    }

    // static values ----------------------------------------------------
    fn static_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyAny>> {
        let (value, object_type) = self.inner_static(value_id)?;
        let mut parser = ValueParser::new(value.get());
        pyparsing::deserialize_py(py, &mut parser, object_type)
    }

    fn static_set(&self, value_id: u64, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let (val, object_type) = self.inner_static(value_id)?;
        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(value, object_type, &mut creator)?;
        let data = creator.finalize();
        val.set(data, update);
        Ok(())
    }

    // signals ----------------------------------------------------------
    fn signal_set(&self, value_id: u64, value: &Bound<PyAny>) -> PyResult<()> {
        let inner = self.get_inner()?;
        match (
            inner.states.signals.get(&value_id),
            inner.types.get(&value_id),
        ) {
            (Some(val), Some(object_type)) => {
                let mut creator = ValueCreator::new();
                pyparsing::serialize_py(value, object_type, &mut creator)?;
                let data = creator.finalize();
                val.set(data);
                Ok(())
            }
            _ => Err(pyo3::exceptions::PyValueError::new_err(
                "Value ID not found",
            )),
        }
    }

    // signal callbacks -------------------------------------------------
    fn signal_set_register(&self, value_id: u64, register: bool) {
        self.signals.set_register(value_id, register);
    }

    fn signal_get<'py>(
        &self,
        py: Python<'py>,
        last_id: Option<u64>,
    ) -> PyResult<(u64, Bound<'py, PyAny>)> {
        let inner = self.get_inner()?;
        let (id, data) = self.signals.wait_changed_value(last_id);
        match inner.types.get(&id) {
            Some(object_type) => {
                let mut parser = ValueParser::new(data);
                let py_value = pyparsing::deserialize_py(py, &mut parser, object_type)?;
                Ok((id, py_value))
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Value ID not found",
            )),
        }
    }

    fn signalset_to_multi(&self, value_id: u64) {
        self.signals.set_to_multi(value_id);
    }

    fn signal_set_to_single(&self, value_id: u64) {
        self.signals.set_to_single(value_id);
    }

    fn signal_get_logging_id(&self) -> u64 {
        self.signals.get_logging_id()
    }

    // lists ------------------------------------------------------------
    fn list_set(&self, value_id: u64, py_list: &Bound<PyList>, update: bool) -> PyResult<()> {
        let (list, value_type) = self.inner_list(value_id)?;
        let mut vec = Vec::with_capacity(py_list.len());
        for item in py_list.iter() {
            let mut creator = ValueCreator::new();
            pyparsing::serialize_py(&item, value_type, &mut creator)?;
            let data = creator.finalize();
            vec.push(data);
        }
        list.set(vec, update);
        Ok(())
    }

    fn list_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyList>> {
        let (list, value_type) = self.inner_list(value_id)?;
        let vec = list.get();
        let py_list = PyList::empty(py);
        for item in vec.iter() {
            let mut parser = ValueParser::new(item.clone());
            let py_value = pyparsing::deserialize_py(py, &mut parser, value_type)?;
            py_list.append(py_value)?;
        }
        Ok(py_list)
    }

    fn list_set_item(
        &self,
        value_id: u64,
        index: usize,
        item: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let (list, value_type) = self.inner_list(value_id)?;
        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(item, value_type, &mut creator)?;
        let data = creator.finalize();
        list.set_item_py(index, data, update)
            .map_err(|e| PyValueError::new_err(e))?;
        Ok(())
    }

    fn list_get_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        index: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (list, value_type) = self.inner_list(value_id)?;
        let data = list
            .get_item(index)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        let mut parser = ValueParser::new(data);
        let py_value = pyparsing::deserialize_py(py, &mut parser, value_type)?;
        Ok(py_value)
    }

    fn list_del_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        index: usize,
        update: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (list, value_type) = self.inner_list(value_id)?;
        let data = list
            .remove_item(index, update)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        let mut parser = ValueParser::new(data);
        let py_value = pyparsing::deserialize_py(py, &mut parser, value_type)?;
        Ok(py_value)
    }

    fn list_append_item(&self, value_id: u64, item: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let (list, value_type) = self.inner_list(value_id)?;
        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(item, value_type, &mut creator)?;
        let data = creator.finalize();
        list.append_item(data, update);
        Ok(())
    }

    fn list_len(&self, value_id: u64) -> PyResult<usize> {
        let inner = self.get_inner()?;
        match inner.states.lists.get(&value_id) {
            Some(list) => Ok(list.len()),
            _ => Err(pyo3::exceptions::PyValueError::new_err(
                "Value ID not found.",
            )),
        }
    }

    // maps -------------------------------------------------------------
    fn map_set(&self, value_id: u64, py_dict: &Bound<PyDict>, update: bool) -> PyResult<()> {
        let (map, key_type, value_type) = self.inner_map(value_id)?;
        let mut new_map = HashMap::with_capacity(py_dict.len());
        for (key, value) in py_dict.iter() {
            let mut key_creator = ValueCreator::new();
            pyparsing::serialize_py(&key, key_type, &mut key_creator)?;
            let key_data = key_creator.finalize();

            let mut value_creator = ValueCreator::new();
            pyparsing::serialize_py(&value, value_type, &mut value_creator)?;
            let value_data = value_creator.finalize();

            new_map.insert(key_data, value_data);
        }
        map.set(new_map, update);
        Ok(())
    }

    fn map_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyDict>> {
        let (map, key_type, value_type) = self.inner_map(value_id)?;
        let data_map = map.get();
        let py_dict = PyDict::new(py);
        for (key_data, value_data) in data_map.iter() {
            let mut key_parser = ValueParser::new(key_data.clone());
            let py_key = pyparsing::deserialize_py(py, &mut key_parser, key_type)?;

            let mut value_parser = ValueParser::new(value_data.clone());
            let py_value = pyparsing::deserialize_py(py, &mut value_parser, value_type)?;

            py_dict.set_item(py_key, py_value)?;
        }
        Ok(py_dict)
    }

    fn map_set_item(
        &self,
        value_id: u64,
        key: &Bound<PyAny>,
        value: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let (map, key_type, value_type) = self.inner_map(value_id)?;
        let mut key_creator = ValueCreator::new();
        pyparsing::serialize_py(key, key_type, &mut key_creator)?;
        let key_data = key_creator.finalize();

        let mut value_creator = ValueCreator::new();
        pyparsing::serialize_py(value, value_type, &mut value_creator)?;
        let value_data = value_creator.finalize();

        map.set_item(key_data, value_data, update);
        Ok(())
    }

    fn map_get_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        key: &Bound<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (map, key_type, value_type) = self.inner_map(value_id)?;
        let mut key_creator = ValueCreator::new();
        pyparsing::serialize_py(key, key_type, &mut key_creator)?;
        let key_data = key_creator.finalize();
        match map.get_item(&key_data) {
            Some(value_data) => {
                let mut value_parser = ValueParser::new(value_data);
                let py_value = pyparsing::deserialize_py(py, &mut value_parser, value_type)?;
                Ok(py_value)
            }
            None => Err(PyValueError::new_err("Key not found in map.")),
        }
    }

    fn map_del_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        key: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (map, key_type, value_type) = self.inner_map(value_id)?;
        let mut key_creator = ValueCreator::new();
        pyparsing::serialize_py(key, key_type, &mut key_creator)?;
        let key_data = key_creator.finalize();

        match map.remove_item(&key_data, update) {
            Some(value_data) => {
                let mut value_parser = ValueParser::new(value_data);
                let py_value = pyparsing::deserialize_py(py, &mut value_parser, value_type)?;
                Ok(py_value)
            }
            None => Err(PyValueError::new_err("Key not found in map.")),
        }
    }

    fn map_len(&self, value_id: u64) -> PyResult<usize> {
        let inner = self.get_inner()?;
        match inner.states.maps.get(&value_id) {
            Some(map) => Ok(map.len()),
            _ => Err(pyo3::exceptions::PyValueError::new_err(
                "Value ID not found.",
            )),
        }
    }

    // images -----------------------------------------------------------
    fn image_size(&self, value_id: u64) -> PyResult<(usize, usize)> {
        let size = self.inner_image(value_id)?.get_size();
        Ok((size[0], size[1]))
    }

    fn image_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
    ) -> PyResult<(Bound<'py, PyByteArray>, (usize, usize))> {
        let (array, size) = self.inner_image(value_id)?.get_image(|(data, size)| {
            let array = PyByteArray::new(py, data);
            (array, (size[0], size[1]))
        });
        Ok((array, size))
    }

    #[pyo3(signature = (value_id, image, update, origin=None))]
    fn image_set(
        &self,
        py: Python,
        value_id: u64,
        image: PyBuffer<u8>,
        update: bool,
        origin: Option<[u32; 2]>,
    ) -> PyResult<()> {
        py.detach(|| {
            let image_val = self.inner_image(value_id)?;
            pyimage::set_image(&image, image_val, origin, update)
        })
    }

    // graphs -----------------------------------------------------------
    fn graphs_set(
        &self,
        py: Python,
        value_id: u64,
        idx: u16,
        graph: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let graphs = self.inner_graphs(value_id)?;
        match graphs.graph_type() {
            GraphType::F32 => {
                let graph_buffer = PyBuffer::<f32>::extract(graph.as_borrowed())?;
                py.detach(|| {
                    let graph_data = pygraphs::buffer_to_data(&graph_buffer)?;
                    graphs.set(idx, graph_data, update);
                    Ok(())
                })
            }
            GraphType::F64 => {
                let graph_buffer = PyBuffer::<f64>::extract(graph.as_borrowed())?;
                py.detach(|| {
                    let graph_data = pygraphs::buffer_to_data(&graph_buffer)?;
                    graphs.set(idx, graph_data, update);
                    Ok(())
                })
            }
        }
    }

    fn graphs_add_points(
        &self,
        py: Python,
        value_id: u64,
        idx: u16,
        graph: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let graphs = self.inner_graphs(value_id)?;
        match graphs.graph_type() {
            GraphType::F32 => {
                let graph_buffer = PyBuffer::<f32>::extract(graph.as_borrowed())?;
                py.detach(|| {
                    let graph_data = pygraphs::buffer_to_data(&graph_buffer)?;
                    graphs.add_points(idx, graph_data, update).map_err(|e| {
                        PyValueError::new_err(format!("Failed to add points to graph: {}", e))
                    })
                })
            }
            GraphType::F64 => {
                let graph_buffer = PyBuffer::<f64>::extract(graph.as_borrowed())?;
                py.detach(|| {
                    let graph_data = pygraphs::buffer_to_data(&graph_buffer)?;
                    graphs.add_points(idx, graph_data, update).map_err(|e| {
                        PyValueError::new_err(format!("Failed to add points to graph: {}", e))
                    })
                })
            }
        }
    }

    fn graphs_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        idx: u16,
    ) -> PyResult<Bound<'py, PyTuple>> {
        let graphs = self.inner_graphs(value_id)?;
        match graphs.graph_type() {
            GraphType::F32 => graphs
                .get(idx, |data| pygraphs::graph_to_buffer::<f32>(py, data))
                .ok_or_else(|| PyValueError::new_err(format!("No graph found at index {}", idx)))?,
            GraphType::F64 => graphs
                .get(idx, |data| pygraphs::graph_to_buffer::<f64>(py, data))
                .ok_or_else(|| PyValueError::new_err(format!("No graph found at index {}", idx)))?,
        }
    }

    fn graphs_count(&self, value_id: u64) -> PyResult<usize> {
        let count = self.inner_graphs(value_id)?.count();
        Ok(count)
    }

    fn graphs_len(&self, value_id: u64, idx: u16) -> PyResult<usize> {
        let len = self
            .inner_graphs(value_id)?
            .len(idx)
            .ok_or_else(|| PyValueError::new_err(format!("No graph found at index {}", idx)))?;
        Ok(len)
    }

    fn graphs_remove(&self, value_id: u64, idx: u16, update: bool) -> PyResult<()> {
        self.inner_graphs(value_id)?.remove(idx, update);
        Ok(())
    }

    fn graphs_reset(&self, value_id: u64, update: bool) -> PyResult<()> {
        self.inner_graphs(value_id)?.reset(update);
        Ok(())
    }

    fn graphs_is_linear(&self, value_id: u64, idx: u16) -> PyResult<bool> {
        self.inner_graphs(value_id)?
            .is_linear(idx)
            .map_err(|_| PyValueError::new_err(format!("No graph found at index {}", idx)))
    }

    // add states -------------------------------------------------------
    // ------------------------------------------------------------------
    fn add_value(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectType>,
        initial_value: &Bound<PyAny>,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_hash(py)?;

        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(initial_value, &object_type, &mut creator)?;
        let data = creator.finalize();

        let value_id = generate_value_id(&name);
        self.server
            .write()
            .add_value(value_id, type_id, data)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add value: {}", e))
            })?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(type_id, object_type);
        }
        Ok(value_id)
    }

    fn add_static(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectType>,
        initial_value: &Bound<PyAny>,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_hash(py)?;

        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(initial_value, &object_type, &mut creator)?;
        let data = creator.finalize();

        let value_id = generate_value_id(&name);
        self.server
            .write()
            .add_static(value_id, type_id, data)
            .map_err(|e| PyValueError::new_err(format!("Failed to add value: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(type_id, object_type);
        }
        Ok(value_id)
    }

    fn add_signal(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectType>,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_hash(py)?;

        let value_id = generate_value_id(&name);
        self.server
            .write()
            .add_signal(value_id, type_id)
            .map_err(|e| PyValueError::new_err(format!("Failed to add signal: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(type_id, object_type);
        }
        Ok(value_id)
    }

    fn add_list(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectType>,
    ) -> PyResult<u64> {
        let object_type = ObjectType::Vec(Box::new(object_type.borrow().object_type.clone_py(py)));
        let type_id = object_type.get_hash(py)?;

        let value_id = generate_value_id(&name);
        self.server
            .write()
            .add_list(value_id, type_id)
            .map_err(|e| PyValueError::new_err(format!("Failed to add list: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(type_id, object_type);
        }
        Ok(value_id)
    }

    fn add_map(
        &self,
        py: Python,
        name: String,
        key_type: &Bound<PyObjectType>,
        value_type: &Bound<PyObjectType>,
    ) -> PyResult<u64> {
        let key_type = key_type.borrow().object_type.clone_py(py);
        let value_type = value_type.borrow().object_type.clone_py(py);
        let object_type = ObjectType::Map(Box::new(key_type), Box::new(value_type));
        let type_id = object_type.get_hash(py)?;

        let value_id = generate_value_id(&name);
        self.server
            .write()
            .add_map(value_id, type_id)
            .map_err(|e| PyValueError::new_err(format!("Failed to add map: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(type_id, object_type);
        }
        Ok(value_id)
    }

    fn add_image(&self, name: String) -> PyResult<u64> {
        let value_id = generate_value_id(&name);
        self.server
            .write()
            .add_image(value_id)
            .map_err(|e| PyValueError::new_err(format!("Failed to add image: {}", e)))?;
        Ok(value_id)
    }

    fn add_graphs(&self, name: String, is_double: bool) -> PyResult<u64> {
        let graph_type = match is_double {
            true => GraphType::F64,
            false => GraphType::F32,
        };

        let value_id = generate_value_id(&name);
        self.server
            .write()
            .add_graphs(value_id, graph_type)
            .map_err(|e| PyValueError::new_err(format!("Failed to add graphs: {}", e)))?;
        Ok(value_id)
    }
}
