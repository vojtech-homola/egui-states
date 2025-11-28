use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::OnceLock;

use pyo3::buffer::PyBuffer;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyDict, PyList};

use egui_states_core::graphs::GraphType;
use egui_states_core::nohash::NoHashMap;
use egui_states_core::types::ObjectType;

use crate::python::pyimage;
use crate::python::pyparsing;
use crate::python::pytypes::PyObjectType;
use crate::server::{Server, StatesList};
use crate::signals::ChangedValues;
use crate::value_parsing::{ValueCreator, ValueParser};

struct CoreInner {
    states: StatesList,
    signals: ChangedValues,
    types: NoHashMap<u64, ObjectType>,
}

#[pyclass]
pub struct StateServerCore {
    server: RwLock<Server>,
    inner: OnceLock<CoreInner>,
    types_temp: RwLock<Option<NoHashMap<u64, ObjectType>>>,
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
        Ok(Self {
            server: RwLock::new(server),
            inner: OnceLock::new(),
            types_temp: RwLock::new(Some(NoHashMap::default())),
        })
    }

    fn initialize(&self) {
        let states = self.server.write().initialize();
        if let Some((states, signals)) = states {
            if let Some(types_map) = self.types_temp.write().take() {
                let _ = self.inner.set(CoreInner {
                    states,
                    signals,
                    types: types_map,
                });
            }
        }
    }

    fn start(&self) {
        self.server.write().start();
    }

    fn stop(&self) {
        self.server.write().stop();
    }

    fn disconnect_clients(&self) {
        self.server.write().disconnect_client();
    }

    fn is_running(&self) -> bool {
        self.server.read().is_running()
    }
    // values -----------------------------------------------------------
    fn get_value<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyAny>> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.values.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(value), Some(object_type)) => {
                        let mut parser = ValueParser::new(value.get());
                        pyparsing::deserelialize_py(py, &mut parser, object_type)
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn set_value(
        &self,
        value_id: u64,
        value: &Bound<PyAny>,
        set_signal: bool,
        update: bool,
    ) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.values.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(val), Some(object_type)) => {
                        let mut creator = ValueCreator::new();
                        pyparsing::serialize_py(value, object_type, &mut creator)?;
                        let data = creator.finalize();
                        val.set(data, set_signal, update);
                        Ok(())
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    // static values ----------------------------------------------------
    fn set_value_static(&self, value_id: u64, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.static_values.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(val), Some(object_type)) => {
                        let mut creator = ValueCreator::new();
                        pyparsing::serialize_py(value, object_type, &mut creator)?;
                        let data = creator.finalize();
                        val.set(data, update);
                        Ok(())
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    // signals ----------------------------------------------------------
    fn set_signal(&self, value_id: u64, value: &Bound<PyAny>) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => {
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
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    // signal callbacks -------------------------------------------------
    fn signal_set_register(&self, value_id: u64, register: bool) {
        if let Some(inner) = self.inner.get() {
            inner.signals.set_registerd(value_id, register);
        }
    }

    fn get_signal<'py>(
        &self,
        py: Python<'py>,
        last_id: Option<u64>,
    ) -> PyResult<(u64, Bound<'py, PyAny>)> {
        match self.inner.get() {
            Some(inner) => {
                let (id, data) = inner.signals.wait_changed_value(last_id);
                match inner.types.get(&id) {
                    Some(object_type) => {
                        let mut parser = ValueParser::new(data);
                        let py_value = pyparsing::deserelialize_py(py, &mut parser, object_type)?;
                        Ok((id, py_value))
                    }
                    None => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn set_to_multi(&self, value_id: u64) {
        if let Some(inner) = self.inner.get() {
            inner.signals.set_to_multi(value_id);
        }
    }

    fn set_to_single(&self, value_id: u64) {
        if let Some(inner) = self.inner.get() {
            inner.signals.set_to_single(value_id);
        }
    }

    // lists ------------------------------------------------------------
    fn list_set(&self, value_id: u64, py_list: &Bound<PyList>, update: bool) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.lists.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(list), Some(ObjectType::Vec(value_type))) => {
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
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found or type mismatch.",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn list_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyList>> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.lists.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(list), Some(ObjectType::Vec(value_type))) => {
                        let vec = list.get();
                        let py_list = PyList::empty(py);
                        for item in vec.iter() {
                            let mut parser = ValueParser::new(item.clone());
                            let py_value =
                                pyparsing::deserelialize_py(py, &mut parser, value_type)?;
                            py_list.append(py_value)?;
                        }
                        Ok(py_list)
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found or type mismatch.",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn list_set_item(
        &self,
        value_id: u64,
        index: usize,
        item: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.lists.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(list), Some(ObjectType::Vec(value_type))) => {
                        let mut creator = ValueCreator::new();
                        pyparsing::serialize_py(item, value_type, &mut creator)?;
                        let data = creator.finalize();
                        list.set_item_py(index, data, update)
                            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                        Ok(())
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found or type mismatch.",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn list_get_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        index: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.lists.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(list), Some(ObjectType::Vec(value_type))) => {
                        let data = list
                            .get_item(index)
                            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                        let mut parser = ValueParser::new(data);
                        let py_value = pyparsing::deserelialize_py(py, &mut parser, value_type)?;
                        Ok(py_value)
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found or type mismatch.",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn list_remove_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        index: usize,
        update: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.lists.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(list), Some(ObjectType::Vec(value_type))) => {
                        let data = list
                            .remove_item(index, update)
                            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                        let mut parser = ValueParser::new(data);
                        let py_value = pyparsing::deserelialize_py(py, &mut parser, value_type)?;
                        Ok(py_value)
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found or type mismatch.",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn list_append_item(&self, value_id: u64, item: &Bound<PyAny>, update: bool) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => {
                match (
                    inner.states.lists.get(&value_id),
                    inner.types.get(&value_id),
                ) {
                    (Some(list), Some(ObjectType::Vec(value_type))) => {
                        let mut creator = ValueCreator::new();
                        pyparsing::serialize_py(item, value_type, &mut creator)?;
                        let data = creator.finalize();
                        list.append_item(data, update);
                        Ok(())
                    }
                    _ => Err(pyo3::exceptions::PyValueError::new_err(
                        "Value ID not found or type mismatch.",
                    )),
                }
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn list_len(&self, value_id: u64) -> PyResult<usize> {
        match self.inner.get() {
            Some(inner) => match inner.states.lists.get(&value_id) {
                Some(list) => Ok(list.len()),
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    // maps -------------------------------------------------------------
    fn map_set(&self, value_id: u64, py_dict: &Bound<PyDict>, update: bool) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => match (inner.states.maps.get(&value_id), inner.types.get(&value_id)) {
                (Some(map), Some(ObjectType::Map(key_type, value_type))) => {
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
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found or type mismatch.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn map_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyDict>> {
        match self.inner.get() {
            Some(inner) => match (inner.states.maps.get(&value_id), inner.types.get(&value_id)) {
                (Some(map), Some(ObjectType::Map(key_type, value_type))) => {
                    let data_map = map.get();
                    let py_dict = PyDict::new(py);
                    for (key_data, value_data) in data_map.iter() {
                        let mut key_parser = ValueParser::new(key_data.clone());
                        let py_key = pyparsing::deserelialize_py(py, &mut key_parser, key_type)?;

                        let mut value_parser = ValueParser::new(value_data.clone());
                        let py_value =
                            pyparsing::deserelialize_py(py, &mut value_parser, value_type)?;

                        py_dict.set_item(py_key, py_value)?;
                    }
                    Ok(py_dict)
                }
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found or type mismatch.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn map_set_item(
        &self,
        value_id: u64,
        key: &Bound<PyAny>,
        item: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => match (inner.states.maps.get(&value_id), inner.types.get(&value_id)) {
                (Some(map), Some(ObjectType::Map(key_type, value_type))) => {
                    let mut key_creator = ValueCreator::new();
                    pyparsing::serialize_py(key, key_type, &mut key_creator)?;
                    let key_data = key_creator.finalize();

                    let mut value_creator = ValueCreator::new();
                    pyparsing::serialize_py(item, value_type, &mut value_creator)?;
                    let value_data = value_creator.finalize();

                    map.set_item(key_data, value_data, update);
                    Ok(())
                }
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found or type mismatch.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn map_get_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        key: &Bound<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        match self.inner.get() {
            Some(inner) => match (inner.states.maps.get(&value_id), inner.types.get(&value_id)) {
                (Some(map), Some(ObjectType::Map(key_type, value_type))) => {
                    let mut key_creator = ValueCreator::new();
                    pyparsing::serialize_py(key, key_type, &mut key_creator)?;
                    let key_data = key_creator.finalize();

                    match map.get_item(&key_data) {
                        Some(value_data) => {
                            let mut value_parser = ValueParser::new(value_data);
                            let py_value =
                                pyparsing::deserelialize_py(py, &mut value_parser, value_type)?;
                            Ok(py_value)
                        }
                        None => Err(pyo3::exceptions::PyValueError::new_err(
                            "Key not found in map.",
                        )),
                    }
                }
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found or type mismatch.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn map_remove_item<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        key: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        match self.inner.get() {
            Some(inner) => match (inner.states.maps.get(&value_id), inner.types.get(&value_id)) {
                (Some(map), Some(ObjectType::Map(key_type, value_type))) => {
                    let mut key_creator = ValueCreator::new();
                    pyparsing::serialize_py(key, key_type, &mut key_creator)?;
                    let key_data = key_creator.finalize();

                    match map.remove_item(&key_data, update) {
                        Ok(value_data) => {
                            let mut value_parser = ValueParser::new(value_data);
                            let py_value =
                                pyparsing::deserelialize_py(py, &mut value_parser, value_type)?;
                            Ok(py_value)
                        }
                        Err(e) => Err(pyo3::exceptions::PyValueError::new_err(e)),
                    }
                }
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found or type mismatch.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn map_len(&self, value_id: u64) -> PyResult<usize> {
        match self.inner.get() {
            Some(inner) => match inner.states.maps.get(&value_id) {
                Some(map) => Ok(map.len()),
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    // images -----------------------------------------------------------
    fn image_get_size(&self, value_id: u64) -> PyResult<(usize, usize)> {
        match self.inner.get() {
            Some(inner) => match inner.states.images.get(&value_id) {
                Some(image) => {
                    let size = image.get_size();
                    Ok((size[0], size[1]))
                }
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn image_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
    ) -> PyResult<(Bound<'py, PyByteArray>, (usize, usize))> {
        match self.inner.get() {
            Some(inner) => match inner.states.images.get(&value_id) {
                Some(image) => {
                    let (array, size) = image.get_image(|(data, size)| {
                        let array = PyByteArray::new(py, data);
                        (array, (size[0], size[1]))
                    });
                    Ok((array, size))
                }
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    fn image_set(
        &self,
        py: Python,
        value_id: u64,
        image: PyBuffer<u8>,
        origin: Option<[u32; 2]>,
        update: bool,
    ) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => match inner.states.images.get(&value_id) {
                Some(image_value) => {
                    py.detach(|| pyimage::set_image(&image, image_value, origin, update))
                }
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    // graphs -----------------------------------------------------------
    fn graph_set(
        &self,
        py: Python,
        value_id: u64,
        idx: u16,
        graph: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        match self.inner.get() {
            Some(inner) => match inner.states.graphs.get(&value_id) {
                Some(graphs) => match graphs.graph_type() {
                    GraphType::F32 => {
                        let graph_buffer = PyBuffer::<f32>::extract(graph.as_borrowed())?;
                        py.detach(|| {
                            crate::python::pygraphs::set_graph(idx, &graph_buffer, graphs, update)
                        })
                    }
                    GraphType::F64 => {
                        let graph_buffer = PyBuffer::<f64>::extract(graph.as_borrowed())?;
                        py.detach(|| {
                            crate::python::pygraphs::set_graph(idx, &graph_buffer, graphs, update)
                        })
                    }
                },
                _ => Err(pyo3::exceptions::PyValueError::new_err(
                    "Value ID not found.",
                )),
            },
            None => Err(pyo3::exceptions::PyValueError::new_err(
                "Server not initialized",
            )),
        }
    }

    // add states -------------------------------------------------------
    fn add_value(
        &self,
        value_id: u64,
        type_id: u64,
        object_type: &Bound<PyObjectType>,
        initial_value: &Bound<PyAny>,
    ) -> PyResult<()> {
        let object_type = object_type.borrow().object_type.clone();

        if type_id != object_type.get_hash() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Type ID does not match the hash of the object type",
            ));
        }

        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(initial_value, &object_type, &mut creator)?;
        let data = creator.finalize();

        self.server
            .write()
            .add_value(value_id, type_id, data)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add value: {}", e))
            })?;

        if let Some(types_map) = self.types_temp.write().as_mut() {
            types_map.insert(type_id, object_type);
        }

        Ok(())
    }

    fn add_static(
        &self,
        value_id: u64,
        type_id: u64,
        object_type: &Bound<PyObjectType>,
        initial_value: &Bound<PyAny>,
    ) -> PyResult<()> {
        let object_type = object_type.borrow().object_type.clone();

        if type_id != object_type.get_hash() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Type ID does not match the hash of the object type",
            ));
        }

        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(initial_value, &object_type, &mut creator)?;
        let data = creator.finalize();

        self.server
            .write()
            .add_static(value_id, type_id, data)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add value: {}", e))
            })?;

        if let Some(types_map) = self.types_temp.write().as_mut() {
            types_map.insert(type_id, object_type);
        }

        Ok(())
    }

    fn add_signal(
        &self,
        value_id: u64,
        type_id: u64,
        object_type: &Bound<PyObjectType>,
    ) -> PyResult<()> {
        let object_type = object_type.borrow().object_type.clone();

        if type_id != object_type.get_hash() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Type ID does not match the hash of the object type",
            ));
        }

        self.server
            .write()
            .add_signal(value_id, type_id)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add signal: {}", e))
            })?;

        if let Some(types_map) = self.types_temp.write().as_mut() {
            types_map.insert(type_id, object_type);
        }

        Ok(())
    }

    fn add_list(
        &self,
        value_id: u64,
        type_id: u64,
        object_type: &Bound<PyObjectType>,
    ) -> PyResult<()> {
        let object_type = object_type.borrow().object_type.clone();

        if type_id != object_type.get_hash() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Type ID does not match the hash of the object type",
            ));
        }

        self.server
            .write()
            .add_list(value_id, type_id)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add list: {}", e))
            })?;

        if let Some(types_map) = self.types_temp.write().as_mut() {
            types_map.insert(type_id, object_type);
        }

        Ok(())
    }

    fn add_map(
        &self,
        value_id: u64,
        type_id: u64,
        key_type: &Bound<PyObjectType>,
        value_type: &Bound<PyObjectType>,
    ) -> PyResult<()> {
        let key_type = key_type.borrow().object_type.clone();
        let value_type = value_type.borrow().object_type.clone();
        let hash = key_type.get_hash_add(&value_type);

        let object_type = ObjectType::Map(Box::new(key_type), Box::new(value_type));

        if type_id != hash {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Type ID does not match the hash of the object type",
            ));
        }

        self.server
            .write()
            .add_map(value_id, type_id)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add map: {}", e))
            })?;

        if let Some(types_map) = self.types_temp.write().as_mut() {
            types_map.insert(type_id, object_type);
        }

        Ok(())
    }

    fn add_image(&self, value_id: u64) -> PyResult<()> {
        self.server.write().add_image(value_id).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Failed to add image: {}", e))
        })?;
        Ok(())
    }

    fn add_graphs(&self, value_id: u64, is_double: bool) -> PyResult<()> {
        let graph_type = match is_double {
            true => GraphType::F64,
            false => GraphType::F32,
        };

        self.server
            .write()
            .add_graphs(value_id, graph_type)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add graphs: {}", e))
            })?;
        Ok(())
    }
}
