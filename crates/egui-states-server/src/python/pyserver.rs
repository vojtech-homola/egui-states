use parking_lot::RwLock;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::OnceLock;

use pyo3::prelude::*;

use egui_states_core::graphs::GraphType;
use egui_states_core::nohash::NoHashMap;
use egui_states_core::values::ObjectType;

use crate::python::pyparsing;
use crate::python::type_creator::PyObjectType;
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

    // maps -------------------------------------------------------------

    // images -----------------------------------------------------------

    // graphs -----------------------------------------------------------

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
