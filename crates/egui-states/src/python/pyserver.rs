use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::sync::OnceLock;

use pyo3::buffer::{PyBuffer, PyUntypedBuffer};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyDict, PyList};

use crate::hashing::NoHashMap;
use crate::python::{
    pydata::check_data_type,
    pyimage, pyparsing,
    pytypes::{PyObjectClass, PyObjectType},
};
use crate::server::data_server::{Data, DataHolder, DataMulti};
use crate::server::data_take_server::{DataMultiTake, DataTake};
use crate::server::server::Server;
use crate::server::signals::SignalsManager;
use crate::server::value_parsing::{ValueCreator, ValueParser};
use crate::server::values_server::{Signal, Value, ValueStatic, ValueTake};
use crate::server::{image_server::ValueImage, map_server::ValueMap, vec_server::ValueList};

struct ValuesInner {
    values: NoHashMap<u64, (Arc<Value>, PyObjectType)>,
    values_take: NoHashMap<u64, (Arc<ValueTake>, PyObjectType)>,
    static_values: NoHashMap<u64, (Arc<ValueStatic>, PyObjectType)>,
    signals: NoHashMap<u64, (Arc<Signal>, PyObjectType)>,
    signals_types: NoHashMap<u64, PyObjectType>,
    maps: NoHashMap<u64, (Arc<ValueMap>, PyObjectType)>,
    lists: NoHashMap<u64, (Arc<ValueList>, PyObjectType)>,
    images: NoHashMap<u64, Arc<ValueImage>>,
    data: NoHashMap<u64, Arc<Data>>,
    data_take: NoHashMap<u64, Arc<DataTake>>,
    data_multi: NoHashMap<u64, Arc<DataMulti>>,
    data_multi_take: NoHashMap<u64, Arc<DataMultiTake>>,
}

#[pyclass]
pub(crate) struct StateServerCore {
    server: RwLock<Server>,
    signals: SignalsManager,
    inner: OnceLock<ValuesInner>,
    temps: RwLock<Option<NoHashMap<u64, PyObjectType>>>,
}

impl StateServerCore {
    #[inline]
    fn get_values(&self) -> PyResult<&ValuesInner> {
        match self.inner.get() {
            Some(inner) => Ok(inner),
            None => Err(PyValueError::new_err("Server is not initialized.")),
        }
    }

    #[inline]
    fn inner_values(&self, value_id: u64) -> PyResult<(&Arc<Value>, &PyObjectType)> {
        match self.get_values()?.values.get(&value_id) {
            Some((value, object_type)) => Ok((value, object_type)),
            _ => Err(PyValueError::new_err("Value with ID not found.")),
        }
    }

    #[inline]
    fn inner_static(&self, value_id: u64) -> PyResult<(&Arc<ValueStatic>, &PyObjectType)> {
        match self.get_values()?.static_values.get(&value_id) {
            Some((value, object_type)) => Ok((value, object_type)),
            _ => Err(PyValueError::new_err("Static value with ID not found.")),
        }
    }

    #[inline]
    fn inner_vec(&self, value_id: u64) -> PyResult<(&Arc<ValueList>, &PyObjectType)> {
        match self.get_values()?.lists.get(&value_id) {
            Some((list, value_type)) => Ok((list, value_type)),
            _ => Err(PyValueError::new_err("Vec with ID not found.")),
        }
    }

    #[inline]
    fn inner_map(&self, value_id: u64) -> PyResult<(&Arc<ValueMap>, &PyObjectType, &PyObjectType)> {
        match self.get_values()?.maps.get(&value_id) {
            Some((map, PyObjectType::Map(key_type, value_type))) => Ok((map, key_type, value_type)),
            _ => Err(PyValueError::new_err(
                "Map with ID not found or type mismatch.",
            )),
        }
    }

    #[inline]
    fn inner_image(&self, value_id: u64) -> PyResult<&Arc<ValueImage>> {
        match self.get_values()?.images.get(&value_id) {
            Some(image) => Ok(image),
            _ => Err(PyValueError::new_err("Image with ID not found.")),
        }
    }

    #[inline]
    fn inner_data(&self, value_id: u64) -> PyResult<&Arc<Data>> {
        match self.get_values()?.data.get(&value_id) {
            Some(data) => Ok(data),
            _ => Err(PyValueError::new_err("Data with ID not found.")),
        }
    }

    #[inline]
    fn inner_data_multi(&self, value_id: u64) -> PyResult<&Arc<DataMulti>> {
        match self.get_values()?.data_multi.get(&value_id) {
            Some(data_multi) => Ok(data_multi),
            _ => Err(PyValueError::new_err("DataMulti with ID not found.")),
        }
    }

    #[inline]
    fn inner_data_take(&self, value_id: u64) -> PyResult<&Arc<DataTake>> {
        match self.get_values()?.data_take.get(&value_id) {
            Some(data_take) => Ok(data_take),
            _ => Err(PyValueError::new_err("DataTake with ID not found.")),
        }
    }

    #[inline]
    fn inner_data_multi_take(&self, value_id: u64) -> PyResult<&Arc<DataMultiTake>> {
        match self.get_values()?.data_multi_take.get(&value_id) {
            Some(data_multi_take) => Ok(data_multi_take),
            _ => Err(PyValueError::new_err("DataMultiTake with ID not found.")),
        }
    }
}

#[pymethods]
impl StateServerCore {
    #[new]
    #[pyo3(signature = (port, ip_addr=None, handshake=None, runner_threads=3))]
    fn new(
        port: u16,
        ip_addr: Option<[u8; 4]>,
        handshake: Option<Vec<u64>>,
        runner_threads: usize,
    ) -> PyResult<Self> {
        let addr = match ip_addr {
            Some(addr) => {
                SocketAddrV4::new(Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]), port)
            }
            None => SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port),
        };

        let server = Server::new(addr, handshake, runner_threads);
        let signals = server.get_signals_manager();

        // register logging signal type
        let logging_object_type = PyObjectType::Tuple(vec![PyObjectType::U8, PyObjectType::String]);
        let mut types = NoHashMap::default();
        types.insert(signals.get_logging_id(), logging_object_type);

        Ok(Self {
            server: RwLock::new(server),
            signals,
            inner: OnceLock::new(),
            temps: RwLock::new(Some(types)),
        })
    }

    fn finalize(&self, py: Python) -> PyResult<()> {
        match (self.server.write().finalize(), self.temps.write().take()) {
            (Some(states), Some(mut types)) => {
                let mut values = NoHashMap::default();
                for (id, state) in states.values {
                    if let Some(object_type) = types.get(&id) {
                        values.insert(id, (state, object_type.clone_py(py)));
                    } else {
                        return Err(PyValueError::new_err(format!(
                            "Missing type information for value ID {}",
                            id
                        )));
                    }
                }

                let mut values_take = NoHashMap::default();
                for (id, state) in states.values_take {
                    if let Some(object_type) = types.get(&id) {
                        values_take.insert(id, (state, object_type.clone_py(py)));
                    } else {
                        return Err(PyValueError::new_err(format!(
                            "Missing type information for value take ID {}",
                            id
                        )));
                    }
                }

                let mut static_values = NoHashMap::default();
                for (id, state) in states.static_values {
                    if let Some(object_type) = types.remove(&id) {
                        static_values.insert(id, (state, object_type));
                    } else {
                        return Err(PyValueError::new_err(format!(
                            "Missing type information for static value ID {}",
                            id
                        )));
                    }
                }

                let mut signals = NoHashMap::default();
                for (id, signal) in states.signals {
                    if let Some(object_type) = types.get(&id) {
                        signals.insert(id, (signal, object_type.clone_py(py)));
                    } else {
                        return Err(PyValueError::new_err(format!(
                            "Missing type information for signal ID {}",
                            id
                        )));
                    }
                }

                let mut maps = NoHashMap::default();
                for (id, map) in states.maps {
                    if let Some(object_type) = types.remove(&id) {
                        maps.insert(id, (map, object_type));
                    } else {
                        return Err(PyValueError::new_err(format!(
                            "Missing type information for map ID {}",
                            id
                        )));
                    }
                }

                let mut lists = NoHashMap::default();
                for (id, list) in states.lists {
                    if let Some(object_type) = types.remove(&id) {
                        lists.insert(id, (list, object_type));
                    } else {
                        return Err(PyValueError::new_err(format!(
                            "Missing type information for list ID {}",
                            id
                        )));
                    }
                }

                let images = states.images;
                let data = states.data;
                let data_take = states.data_take;
                let data_multi = states.data_multi;
                let data_multi_take = states.data_multi_take;

                let inner = ValuesInner {
                    values,
                    values_take,
                    static_values,
                    signals,
                    signals_types: types,
                    maps,
                    lists,
                    images,
                    data,
                    data_take,
                    data_multi,
                    data_multi_take,
                };

                if self.inner.set(inner).is_err() {
                    return Err(PyValueError::new_err("Server has already been finalized."));
                }

                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(PyValueError::new_err(
                "Inconsistent state during finalization.",
            )),
        }
    }

    fn start(&self) -> PyResult<()> {
        self.server
            .write()
            .start()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
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

    fn disconnect_client(&self) {
        self.server.write().disconnect_client();
    }

    fn update(&self, duration: Option<f32>) -> PyResult<()> {
        self.server
            .write()
            .update(duration)
            .map_err(|_| PyRuntimeError::new_err("Update failed."))
    }

    fn id_to_name(&self, value_id: u64) -> PyResult<String> {
        let values = self.get_values()?;
        if let Some((value, _)) = values.values.get(&value_id) {
            return Ok(value.name.clone());
        }
        if let Some((value, _)) = values.values_take.get(&value_id) {
            return Ok(value.name.clone());
        }
        if let Some((value, _)) = values.static_values.get(&value_id) {
            return Ok(value.name.clone());
        }
        if let Some((signal, _)) = values.signals.get(&value_id) {
            return Ok(signal.name.clone());
        }
        if let Some((map, _)) = values.maps.get(&value_id) {
            return Ok(map.name.clone());
        }
        if let Some((list, _)) = values.lists.get(&value_id) {
            return Ok(list.name.clone());
        }
        if let Some(image) = values.images.get(&value_id) {
            return Ok(image.name.clone());
        }
        if let Some(data) = values.data.get(&value_id) {
            return Ok(data.name.clone());
        }
        if let Some(data_multi) = values.data_multi.get(&value_id) {
            return Ok(data_multi.name.clone());
        }
        if let Some(data_take) = values.data_take.get(&value_id) {
            return Ok(data_take.name.clone());
        }
        if let Some(data_multi_take) = values.data_multi_take.get(&value_id) {
            return Ok(data_multi_take.name.clone());
        }

        Err(PyRuntimeError::new_err("Value not found."))
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
        val.set(data, set_signal, update)
            .map_err(|_| PyRuntimeError::new_err("Value set failed."))
    }

    // values take ------------------------------------------------------
    fn value_take_set(
        &self,
        value_id: u64,
        value: &Bound<PyAny>,
        blocking: bool,
        update: bool,
    ) -> PyResult<()> {
        let (val, object_type) = match self.get_values()?.values_take.get(&value_id) {
            Some((value, object_type)) => Ok((value, object_type)),
            _ => Err(PyValueError::new_err("ValueTake with ID not found.")),
        }?;
        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(value, object_type, &mut creator)?;
        let data = creator.finalize();
        val.set(data, blocking, update)
            .map_err(|_| PyRuntimeError::new_err("ValueTake set failed."))
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
        val.set(data, update)
            .map_err(|_| PyRuntimeError::new_err("Static value set failed."))
    }

    // signals ----------------------------------------------------------
    fn signal_set(&self, value_id: u64, value: &Bound<PyAny>) -> PyResult<()> {
        match self.get_values()?.signals.get(&value_id) {
            Some((val, object_type)) => {
                let mut creator = ValueCreator::new();
                pyparsing::serialize_py(value, object_type, &mut creator)?;
                let data = creator.finalize();
                val.set(data);
                Ok(())
            }
            _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Signal with ID {} not found",
                value_id
            ))),
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
        let (id, data) = py.detach(|| self.signals.wait_changed_value(last_id));
        match self.get_values()?.signals_types.get(&id) {
            Some(object_type) => {
                let mut parser = ValueParser::new(data);
                let py_value = pyparsing::deserialize_py(py, &mut parser, object_type)?;
                Ok((id, py_value))
            }
            None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Signal with ID {} not found",
                id
            ))),
        }
    }

    fn signal_set_to_queue(&self, value_id: u64) {
        self.signals.set_to_queue(value_id);
    }

    fn signal_set_to_single(&self, value_id: u64) {
        self.signals.set_to_single(value_id);
    }

    fn signal_get_logging_id(&self) -> u64 {
        self.signals.get_logging_id()
    }

    // lists ------------------------------------------------------------
    fn list_set(&self, value_id: u64, py_list: &Bound<PyList>, update: bool) -> PyResult<()> {
        let (list, value_type) = self.inner_vec(value_id)?;
        let mut vec = Vec::with_capacity(py_list.len());
        for item in py_list.iter() {
            let mut creator = ValueCreator::new();
            pyparsing::serialize_py(&item, value_type, &mut creator)?;
            let data = creator.finalize();
            vec.push(data);
        }
        list.set(vec, update)
            .map_err(|_| PyRuntimeError::new_err("Failed to set list."))
    }

    fn list_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyList>> {
        let (list, value_type) = self.inner_vec(value_id)?;
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
        let (list, value_type) = self.inner_vec(value_id)?;
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
        let (list, value_type) = self.inner_vec(value_id)?;
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
        let (list, value_type) = self.inner_vec(value_id)?;
        let data = list
            .remove_item(index, update)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        let mut parser = ValueParser::new(data);
        let py_value = pyparsing::deserialize_py(py, &mut parser, value_type)?;
        Ok(py_value)
    }

    fn list_append_item(&self, value_id: u64, item: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let (list, value_type) = self.inner_vec(value_id)?;
        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(item, value_type, &mut creator)?;
        let data = creator.finalize();
        list.append_item(data, update)
            .map_err(|_| PyRuntimeError::new_err("Failed to append item to list."))
    }

    fn list_len(&self, value_id: u64) -> PyResult<usize> {
        match self.get_values()?.lists.get(&value_id) {
            Some((list, _)) => Ok(list.len()),
            _ => Err(pyo3::exceptions::PyValueError::new_err(
                "List with ID not found.",
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
        map.set(new_map, update)
            .map_err(|_| PyRuntimeError::new_err("Failed to set map."))
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

        map.set_item(key_data, value_data, update)
            .map_err(|_| PyRuntimeError::new_err("Failed to set item in map."))
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

        match map
            .remove_item(&key_data, update)
            .map_err(|_| PyRuntimeError::new_err("Failed to remove item from map."))?
        {
            Some(value_data) => {
                let mut value_parser = ValueParser::new(value_data);
                let py_value = pyparsing::deserialize_py(py, &mut value_parser, value_type)?;
                Ok(py_value)
            }
            None => Err(PyValueError::new_err("Key not found in map.")),
        }
    }

    fn map_len(&self, value_id: u64) -> PyResult<usize> {
        match self.get_values()?.maps.get(&value_id) {
            Some((map, _)) => Ok(map.len()),
            _ => Err(pyo3::exceptions::PyValueError::new_err(
                "Map with ID not found.",
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

    // data -------------------------------------------------------------
    fn data_get<'py>(&self, py: Python<'py>, value_id: u64) -> PyResult<Bound<'py, PyByteArray>> {
        Ok(self
            .inner_data(value_id)?
            .get(|data| PyByteArray::new(py, data)))
    }

    fn data_set(
        &self,
        py: Python,
        value_id: u64,
        data: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .set(data_holder, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_add(
        &self,
        py: Python,
        value_id: u64,
        data: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .add(data_holder, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_replace(
        &self,
        py: Python,
        value_id: u64,
        data: &Bound<PyAny>,
        index: usize,
        update: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .replace(data_holder, index, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_remove(
        &self,
        py: Python,
        value_id: u64,
        index: usize,
        count: usize,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            self.inner_data(value_id)?
                .remove(index, count, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_clear(&self, py: Python, value_id: u64, update: bool) -> PyResult<()> {
        py.detach(|| {
            self.inner_data(value_id)?
                .clear(update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    // data take -------------------------------------------------------
    fn data_take_set(
        &self,
        py: Python,
        value_id: u64,
        data: &Bound<PyAny>,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data_take(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .set(data_holder, blocking, update, cache)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    // data multi -------------------------------------------------------
    fn data_multi_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        index: u32,
    ) -> PyResult<Bound<'py, PyByteArray>> {
        let data_multi = self.inner_data_multi(value_id)?;
        data_multi.get(index, |data| match data {
            Some(data) => Ok(PyByteArray::new(py, data)),
            None => Err(PyValueError::new_err("DataMulti index not found.")),
        })
    }

    fn data_multi_set(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        data: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data_multi(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .set(index, data_holder, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_multi_add(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        data: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data_multi(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .add(index, data_holder, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_multi_replace(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        data: &Bound<PyAny>,
        data_index: usize,
        update: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data_multi(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .replace(index, data_index, data_holder, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_multi_remove(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        data_index: usize,
        count: usize,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            self.inner_data_multi(value_id)?
                .remove(index, data_index, count, update)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_multi_clear(&self, value_id: u64, index: u32, update: bool) -> PyResult<()> {
        self.inner_data_multi(value_id)?
            .clear(index, update)
            .map_err(|e| PyValueError::new_err(e))
    }

    fn data_multi_remove_index(&self, value_id: u64, index: u32, update: bool) -> PyResult<()> {
        self.inner_data_multi(value_id)?
            .remove_index(index, update)
            .map_err(|e| PyValueError::new_err(e))
    }

    fn data_multi_reset(&self, value_id: u64, update: bool) -> PyResult<()> {
        self.inner_data_multi(value_id)?
            .reset(update)
            .map_err(|e| PyValueError::new_err(e))
    }

    // data multi take --------------------------------------------------
    fn data_multi_take_set(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        data: &Bound<PyAny>,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> PyResult<()> {
        let buffer_untyped = PyUntypedBuffer::get(data)
            .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))?;

        let data_value = self.inner_data_multi_take(value_id)?;
        check_data_type(&buffer_untyped, data_value.data_type)
            .map_err(|e| PyValueError::new_err(e))?;

        let data_holder = DataHolder {
            data: buffer_untyped.buf_ptr() as *const u8,
            count: buffer_untyped.item_count(),
            data_size: buffer_untyped.len_bytes(),
            data_type: data_value.data_type,
        };

        py.detach(|| {
            data_value
                .set(index, data_holder, blocking, update, cache)
                .map_err(|e| PyValueError::new_err(e))
        })
    }

    fn data_multi_take_remove_index(
        &self,
        value_id: u64,
        index: u32,
        update: bool,
    ) -> PyResult<()> {
        self.inner_data_multi_take(value_id)?
            .remove_index(index, update)
            .map_err(|e| PyValueError::new_err(e))
    }

    fn data_multi_take_reset(&self, value_id: u64, update: bool) -> PyResult<()> {
        self.inner_data_multi_take(value_id)?
            .reset(update)
            .map_err(|e| PyValueError::new_err(e))
    }

    // add states -------------------------------------------------------
    // ------------------------------------------------------------------
    fn add_value(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectClass>,
        initial_value: &Bound<PyAny>,
        queue: bool,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_core_type(py)?.get_hash();

        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(initial_value, &object_type, &mut creator)?;
        let data = creator.finalize();

        let value_id = self
            .server
            .write()
            .add_value(&name, type_id, data, queue)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add Value: {}", e))
            })?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(value_id, object_type);
        }
        Ok(value_id)
    }

    fn add_value_take(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectClass>,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_core_type(py)?.get_hash();

        let value_id = self
            .server
            .write()
            .add_value_take(&name, type_id)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("Failed to add ValueTake: {}", e))
            })?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(value_id, object_type);
        }
        Ok(value_id)
    }

    fn add_static(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectClass>,
        initial_value: &Bound<PyAny>,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_hash(py)?;

        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(initial_value, &object_type, &mut creator)?;
        let data = creator.finalize();

        let value_id = self
            .server
            .write()
            .add_static(&name, type_id, data)
            .map_err(|e| PyValueError::new_err(format!("Failed to add Static: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(value_id, object_type);
        }
        Ok(value_id)
    }

    fn add_signal(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectClass>,
        queue: bool,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_hash(py)?;

        let value_id = self
            .server
            .write()
            .add_signal(&name, type_id, queue)
            .map_err(|e| PyValueError::new_err(format!("Failed to add Signal: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(value_id, object_type);
        }
        Ok(value_id)
    }

    fn add_vec(
        &self,
        py: Python,
        name: String,
        object_type: &Bound<PyObjectClass>,
    ) -> PyResult<u64> {
        let object_type = object_type.borrow().object_type.clone_py(py);
        let type_id = object_type.get_hash(py)?;

        let value_id = self
            .server
            .write()
            .add_vec(&name, type_id)
            .map_err(|e| PyValueError::new_err(format!("Failed to add ValueVec: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(value_id, object_type);
        }
        Ok(value_id)
    }

    fn add_map(
        &self,
        py: Python,
        name: String,
        key_type: &Bound<PyObjectClass>,
        value_type: &Bound<PyObjectClass>,
    ) -> PyResult<u64> {
        let key_object_type = key_type.borrow().object_type.clone_py(py);
        let value_object_type = value_type.borrow().object_type.clone_py(py);

        let type_id = key_object_type.get_hash(py)? ^ value_object_type.get_hash(py)?;
        let object_type = PyObjectType::Map(Box::new(key_object_type), Box::new(value_object_type));

        let value_id = self
            .server
            .write()
            .add_map(&name, type_id)
            .map_err(|e| PyValueError::new_err(format!("Failed to add ValueMap: {}", e)))?;

        if let Some(types_map) = self.temps.write().as_mut() {
            types_map.insert(value_id, object_type);
        }
        Ok(value_id)
    }

    fn add_image(&self, name: String) -> PyResult<u64> {
        let value_id = self
            .server
            .write()
            .add_image(&name)
            .map_err(|e| PyValueError::new_err(format!("Failed to add ValueImage: {}", e)))?;
        Ok(value_id)
    }

    fn add_data(&self, name: String, data_type: u8) -> PyResult<u64> {
        let value_id = self
            .server
            .write()
            .add_data(&name, data_type)
            .map_err(|e| PyValueError::new_err(format!("Failed to add Data: {}", e)))?;
        Ok(value_id)
    }

    fn add_data_multi(&self, name: String, data_type: u8) -> PyResult<u64> {
        let value_id = self
            .server
            .write()
            .add_data_multi(&name, data_type)
            .map_err(|e| PyValueError::new_err(format!("Failed to add DataMulti: {}", e)))?;
        Ok(value_id)
    }

    fn add_data_take(&self, name: String, data_type: u8) -> PyResult<u64> {
        let value_id = self
            .server
            .write()
            .add_data_take(&name, data_type)
            .map_err(|e| PyValueError::new_err(format!("Failed to add DataTake: {}", e)))?;
        Ok(value_id)
    }

    fn add_data_multi_take(&self, name: String, data_type: u8) -> PyResult<u64> {
        let value_id = self
            .server
            .write()
            .add_data_multi_take(&name, data_type)
            .map_err(|e| PyValueError::new_err(format!("Failed to add DataMultiTake: {}", e)))?;
        Ok(value_id)
    }
}
