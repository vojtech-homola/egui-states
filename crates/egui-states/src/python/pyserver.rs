use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyDict, PyList};

use crate::ObjectType;
use crate::hashing::NoHashMap;

/// A signal event: id, value, whether it blocks, and the previous value.
type SignalEvent<'py> = (u64, Bound<'py, PyAny>, bool, Option<Bound<'py, PyAny>>);
use crate::python::value_parsing::{ValueCreator, ValueParser};
use crate::python::{
    pydata::{data_buffer, data_slice},
    pyimage, pyparsing,
    pytypes::{PyObjectClass, PyObjectType},
};
use crate::server_core::data_core::{Data, DataMulti};
use crate::server_core::data_take_core::{DataMultiTake, DataTake};
use crate::server_core::heartbeat::{self, Heartbeat};
use crate::server_core::server::{Server, ServerConfig, StateRef};
use crate::server_core::signals::{self, Claim, SignalsManager};
use crate::server_core::values_core::{SignalCore, ValueCore, ValueStaticCore, ValueTakeCore};
use crate::server_core::web::{LoadError, WebSite};
use crate::server_core::{
    image_core::Image, image_multi_core::ImageMulti, map_core::ValueMap, vec_core::ValueList,
};

/// A registered state and, where Python converts its values, their type.
enum PyState {
    Value(Arc<ValueCore>, PyObjectType),
    ValueTake(Arc<ValueTakeCore>, PyObjectType),
    Static(Arc<ValueStaticCore>, PyObjectType),
    Signal(Arc<SignalCore>, PyObjectType),
    Map(Arc<ValueMap>, PyObjectType),
    Vec(Arc<ValueList>, PyObjectType),
    Image(Arc<Image>),
    ImageMulti(Arc<ImageMulti>),
    Data(Arc<Data>),
    DataTake(Arc<DataTake>),
    DataMulti(Arc<DataMulti>),
    DataMultiTake(Arc<DataMultiTake>),
}

impl PyState {
    fn name(&self) -> &str {
        match self {
            Self::Value(state, _) => &state.name,
            Self::ValueTake(state, _) => &state.name,
            Self::Static(state, _) => &state.name,
            Self::Signal(state, _) => &state.name,
            Self::Map(state, _) => &state.name,
            Self::Vec(state, _) => &state.name,
            Self::Image(state) => &state.name,
            Self::ImageMulti(state) => &state.name,
            Self::Data(state) => &state.name,
            Self::DataTake(state) => &state.name,
            Self::DataMulti(state) => &state.name,
            Self::DataMultiTake(state) => &state.name,
        }
    }
}

struct ValuesInner {
    states: NoHashMap<u64, PyState>,
    /// Types of what reaches signal callbacks: values, signals and the
    /// reserved server events.
    signals_types: NoHashMap<u64, PyObjectType>,
}

/// Looks up a state of one kind; a state of another kind is reported like a
/// missing one.
macro_rules! lookup {
    ($self:ident, $id:expr, $pattern:pat => $found:expr, $error:expr) => {
        match $self.get_values()?.states.get(&$id) {
            Some($pattern) => Ok($found),
            _ => Err(PyValueError::new_err($error)),
        }
    };
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
    fn inner_values(&self, value_id: u64) -> PyResult<(&Arc<ValueCore>, &PyObjectType)> {
        lookup!(self, value_id, PyState::Value(value, object_type) => (value, object_type), "Value with ID not found.")
    }

    #[inline]
    fn inner_value_take(&self, value_id: u64) -> PyResult<(&Arc<ValueTakeCore>, &PyObjectType)> {
        lookup!(self, value_id, PyState::ValueTake(value, object_type) => (value, object_type), "ValueTake with ID not found.")
    }

    #[inline]
    fn inner_static(&self, value_id: u64) -> PyResult<(&Arc<ValueStaticCore>, &PyObjectType)> {
        lookup!(self, value_id, PyState::Static(value, object_type) => (value, object_type), "Static value with ID not found.")
    }

    #[inline]
    fn inner_vec(&self, value_id: u64) -> PyResult<(&Arc<ValueList>, &PyObjectType)> {
        lookup!(self, value_id, PyState::Vec(list, value_type) => (list, value_type), "Vec with ID not found.")
    }

    #[inline]
    fn inner_map(&self, value_id: u64) -> PyResult<(&Arc<ValueMap>, &PyObjectType, &PyObjectType)> {
        lookup!(
            self,
            value_id,
            PyState::Map(map, PyObjectType::Map(key_type, value_type)) => (map, &**key_type, &**value_type),
            "Map with ID not found or type mismatch."
        )
    }

    #[inline]
    fn inner_image(&self, value_id: u64) -> PyResult<&Arc<Image>> {
        lookup!(self, value_id, PyState::Image(image) => image, "Image with ID not found.")
    }

    #[inline]
    fn inner_image_multi(&self, value_id: u64) -> PyResult<&Arc<ImageMulti>> {
        lookup!(self, value_id, PyState::ImageMulti(images) => images, "ImageMulti with ID not found.")
    }

    #[inline]
    fn inner_data(&self, value_id: u64) -> PyResult<&Arc<Data>> {
        lookup!(self, value_id, PyState::Data(data) => data, "Data with ID not found.")
    }

    #[inline]
    fn inner_data_multi(&self, value_id: u64) -> PyResult<&Arc<DataMulti>> {
        lookup!(self, value_id, PyState::DataMulti(data) => data, "DataMulti with ID not found.")
    }

    #[inline]
    fn inner_data_take(&self, value_id: u64) -> PyResult<&Arc<DataTake>> {
        lookup!(self, value_id, PyState::DataTake(data) => data, "DataTake with ID not found.")
    }

    #[inline]
    fn inner_data_multi_take(&self, value_id: u64) -> PyResult<&Arc<DataMultiTake>> {
        lookup!(self, value_id, PyState::DataMultiTake(data) => data, "DataMultiTake with ID not found.")
    }
}

#[pymethods]
impl StateServerCore {
    #[new]
    #[pyo3(signature = (
        version=None,
        heartbeat_interval=heartbeat::DEFAULT_INTERVAL.as_secs_f64(),
        heartbeat_timeout=Some(heartbeat::DEFAULT_TIMEOUT.as_secs_f64()),
        web_path=None,
        allowed_origins=Vec::new(),
    ))]
    fn new(
        version: Option<u64>,
        heartbeat_interval: f64,
        heartbeat_timeout: Option<f64>,
        web_path: Option<PathBuf>,
        allowed_origins: Vec<String>,
    ) -> PyResult<Self> {
        let heartbeat = Heartbeat::from_secs(heartbeat_interval, heartbeat_timeout)
            .map_err(PyValueError::new_err)?;
        let config = ServerConfig::new(version, heartbeat, web_path.as_deref(), &allowed_origins)
            .map_err(PyValueError::new_err)?;
        let server = Server::new(config);
        let signals = server.get_signals_manager();

        let mut types = NoHashMap::default();

        // register reserved signal types
        let logging_object_type = PyObjectType::Tuple(vec![PyObjectType::U8, PyObjectType::String]);
        types.insert(signals::LOGGING_ID, logging_object_type);
        types.insert(signals::ON_CONNECT_ID, PyObjectType::String);
        types.insert(signals::ON_DISCONNECT_ID, PyObjectType::Empty);
        types.insert(signals::CLIENT_MESSAGE_ID, PyObjectType::String);

        Ok(Self {
            server: RwLock::new(server),
            signals,
            inner: OnceLock::new(),
            temps: RwLock::new(Some(types)),
        })
    }

    fn finalize(&self, py: Python) -> PyResult<()> {
        let (core_states, mut types) =
            match (self.server.write().finalize(), self.temps.write().take()) {
                (Some(core_states), Some(types)) => (core_states, types),
                (None, None) => return Ok(()),
                _ => {
                    return Err(PyValueError::new_err(
                        "Inconsistent state during finalization.",
                    ));
                }
            };

        let missing = |kind: &str, id: u64| {
            PyValueError::new_err(format!("Missing type information for {kind} ID {id}"))
        };
        let mut states = NoHashMap::default();
        for (id, state) in core_states.states {
            // Values, value takes and signals keep their types for signal
            // callbacks; the other kinds take theirs.
            let state = match state {
                StateRef::Value(state) => PyState::Value(
                    state,
                    types
                        .get(&id)
                        .ok_or_else(|| missing("value", id))?
                        .clone_py(py),
                ),
                StateRef::ValueTake(state) => PyState::ValueTake(
                    state,
                    types
                        .get(&id)
                        .ok_or_else(|| missing("value take", id))?
                        .clone_py(py),
                ),
                StateRef::Signal(state) => PyState::Signal(
                    state,
                    types
                        .get(&id)
                        .ok_or_else(|| missing("signal", id))?
                        .clone_py(py),
                ),
                StateRef::Static(state) => PyState::Static(
                    state,
                    types
                        .remove(&id)
                        .ok_or_else(|| missing("static value", id))?,
                ),
                StateRef::Map(state) => {
                    PyState::Map(state, types.remove(&id).ok_or_else(|| missing("map", id))?)
                }
                StateRef::Vec(state) => {
                    PyState::Vec(state, types.remove(&id).ok_or_else(|| missing("list", id))?)
                }
                StateRef::Image(state) => PyState::Image(state),
                StateRef::ImageMulti(state) => PyState::ImageMulti(state),
                StateRef::Data(state) => PyState::Data(state),
                StateRef::DataTake(state) => PyState::DataTake(state),
                StateRef::DataMulti(state) => PyState::DataMulti(state),
                StateRef::DataMultiTake(state) => PyState::DataMultiTake(state),
            };
            states.insert(id, state);
        }

        let inner = ValuesInner {
            states,
            signals_types: types,
        };
        if self.inner.set(inner).is_err() {
            return Err(PyValueError::new_err("Server has already been finalized."));
        }
        Ok(())
    }

    // The lifecycle calls run without the GIL: `stop` joins the server
    // thread, and the others wait for a `stop` in progress, so callbacks can
    // still call into the server meanwhile.

    #[pyo3(signature = (port, ip_addr=None, token=None))]
    fn start(
        &self,
        py: Python,
        port: u16,
        ip_addr: Option<[u8; 4]>,
        token: Option<String>,
    ) -> PyResult<()> {
        let addr = match ip_addr {
            Some(addr) => {
                SocketAddrV4::new(Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]), port)
            }
            None => SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
        };
        py.detach(|| self.server.read().start(addr, token))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    fn address(&self, py: Python) -> Option<((u8, u8, u8, u8), u16)> {
        // A tuple, not `[u8; 4]`, which PyO3 would convert to `bytes`.
        py.detach(|| self.server.read().address()).map(|addr| {
            let [a, b, c, d] = addr.ip().octets();
            ((a, b, c, d), addr.port())
        })
    }

    fn stop(&self, py: Python) {
        py.detach(|| self.server.read().stop());
    }

    fn is_running(&self, py: Python) -> bool {
        py.detach(|| self.server.read().is_running())
    }

    fn is_connected(&self) -> bool {
        self.server.read().is_connected()
    }

    fn disconnect_client(&self) {
        self.server.read().disconnect_client();
    }

    fn start_web(&self, py: Python) -> PyResult<()> {
        // Read the files without holding the server lock or the GIL.
        let path =
            self.server.read().web_path().ok_or_else(|| {
                PyRuntimeError::new_err("The server was created without web_path.")
            })?;
        let site = py.detach(|| WebSite::load(&path)).map_err(|e| match e {
            // Keeps the kind, so a missing path raises FileNotFoundError.
            LoadError::Io(e) => PyErr::from(e),
            LoadError::Invalid(message) => PyValueError::new_err(message),
        })?;
        self.server.read().set_web(Some(site));
        Ok(())
    }

    fn stop_web(&self) {
        self.server.read().set_web(None);
    }

    fn is_web_running(&self) -> bool {
        self.server.read().is_web_running()
    }

    fn web_path(&self) -> Option<PathBuf> {
        self.server.read().web_path()
    }

    fn update(&self, duration: Option<f32>) -> PyResult<()> {
        self.server
            .read()
            .update(duration)
            .map_err(|_| PyRuntimeError::new_err("Update failed."))
    }

    fn id_to_name(&self, value_id: u64) -> PyResult<String> {
        match self.get_values()?.states.get(&value_id) {
            Some(state) => Ok(state.name().to_string()),
            None => Err(PyRuntimeError::new_err("Value not found.")),
        }
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
            .map_err(PyValueError::new_err)
    }

    // values take ------------------------------------------------------
    fn value_take_set(
        &self,
        py: Python,
        value_id: u64,
        value: &Bound<PyAny>,
        blocking: bool,
        update: bool,
    ) -> PyResult<()> {
        let (val, object_type) = self.inner_value_take(value_id)?;
        let mut creator = ValueCreator::new();
        pyparsing::serialize_py(value, object_type, &mut creator)?;
        let data = creator.finalize();
        py.detach(|| {
            val.set(data, blocking, update)
                .map_err(PyValueError::new_err)
        })
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
        val.set(data, update).map_err(PyValueError::new_err)
    }

    // signals ----------------------------------------------------------
    fn signal_set(&self, value_id: u64, value: &Bound<PyAny>) -> PyResult<()> {
        match self.get_values()?.states.get(&value_id) {
            Some(PyState::Signal(val, object_type)) => {
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
    fn signal_register(&self, value_id: u64, register: bool, with_previous: bool) {
        self.signals.set_register(value_id, register, with_previous);
    }

    /// Waits for the next change and decodes it, as `(id, value, has_previous,
    /// previous)`, or returns `None` once [`Self::signals_close`] was called.
    ///
    /// The replaced value is present only for a `Value` whose registration asked for
    /// it, and is never decoded otherwise. Presence is reported by its own flag rather
    /// than by `previous is None`: an optional state deserializes a perfectly ordinary
    /// previous value of `None`, which would otherwise be indistinguishable from
    /// carrying no previous value at all.
    fn signal_get<'py>(
        &self,
        py: Python<'py>,
        last_id: Option<u64>,
    ) -> PyResult<Option<SignalEvent<'py>>> {
        let Claim::Ready(id, data, previous) = py.detach(|| self.signals.claim(last_id, None))
        else {
            return Ok(None);
        };

        let parsed = match self.get_values() {
            Ok(values) => match values.signals_types.get(&id) {
                Some(object_type) => {
                    let mut parser = ValueParser::new(data);
                    pyparsing::deserialize_py(py, &mut parser, object_type).and_then(|value| {
                        // The new and previous values share the state's single type.
                        match previous {
                            None => Ok((value, None)),
                            Some(previous) => {
                                let mut parser = ValueParser::new(previous);
                                pyparsing::deserialize_py(py, &mut parser, object_type)
                                    .map(|previous| (value, Some(previous)))
                            }
                        }
                    })
                }
                None => Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "Signal with ID {} not found",
                    id
                ))),
            },
            Err(error) => Err(error),
        };

        match parsed {
            Ok((py_value, py_previous)) => {
                let has_previous = py_previous.is_some();
                Ok(Some((id, py_value, has_previous, py_previous)))
            }
            Err(error) => {
                // `id` is blocked until it comes back as the next `last_id`.
                // Failing here means the caller never learns it exists, so
                // release it or that value's signals are stuck for good.
                self.signals.release(id);
                Err(error)
            }
        }
    }

    /// Makes every current and future `signal_get` return `None`, so that
    /// callback workers exit. Called when the owning Python server is released.
    fn signals_close(&self) {
        self.signals.close();
    }

    fn signal_set_to_queue(&self, value_id: u64) {
        self.signals.set_to_queue(value_id);
    }

    fn signal_set_to_single(&self, value_id: u64) {
        self.signals.set_to_single(value_id);
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
        list.set(vec, update).map_err(PyValueError::new_err)
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
            .map_err(PyValueError::new_err)?;
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
            .map_err(pyo3::exceptions::PyValueError::new_err)?;
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
            .map_err(pyo3::exceptions::PyValueError::new_err)?;
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
            .map_err(PyValueError::new_err)
    }

    fn list_len(&self, value_id: u64) -> PyResult<usize> {
        match self.get_values()?.states.get(&value_id) {
            Some(PyState::Vec(list, _)) => Ok(list.len()),
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
        map.set(new_map, update).map_err(PyValueError::new_err)
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
            .map_err(PyValueError::new_err)
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
        match self.get_values()?.states.get(&value_id) {
            Some(PyState::Map(map, _)) => Ok(map.len()),
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

    #[pyo3(signature = (value_id, image, update))]
    fn image_set(
        &self,
        py: Python,
        value_id: u64,
        image: PyBuffer<u8>,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            let image_val = self.inner_image(value_id)?;
            let image_data = pyimage::image_data(&image)?;
            image_val
                .set_image(image_data, update)
                .map_err(PyValueError::new_err)
        })
    }

    #[pyo3(signature = (value_id, shape, color, update))]
    fn image_set_all(
        &self,
        py: Python,
        value_id: u64,
        shape: [usize; 2],
        color: &Bound<'_, PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let rgba = pyimage::image_color(color)?;
        py.detach(|| {
            self.inner_image(value_id)?
                .set_all_image(shape, rgba, update)
                .map_err(PyValueError::new_err)
        })
    }

    #[pyo3(signature = (value_id, image, origin, update, force=false))]
    fn image_update(
        &self,
        py: Python,
        value_id: u64,
        image: PyBuffer<u8>,
        origin: [u32; 2],
        update: bool,
        force: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            let image_val = self.inner_image(value_id)?;
            let image_data = pyimage::image_data(&image)?;
            image_val
                .update_image(
                    &[origin[0] as usize, origin[1] as usize],
                    image_data,
                    update,
                    force,
                )
                .map_err(PyValueError::new_err)
        })
    }

    // image multi ------------------------------------------------------
    fn image_multi_size(&self, value_id: u64, index: u32) -> PyResult<(usize, usize)> {
        let size = self
            .inner_image_multi(value_id)?
            .get_size(index)
            .ok_or_else(|| PyValueError::new_err("ImageMulti index not found."))?;
        Ok((size[0], size[1]))
    }

    fn image_multi_get<'py>(
        &self,
        py: Python<'py>,
        value_id: u64,
        index: u32,
    ) -> PyResult<(Bound<'py, PyByteArray>, (usize, usize))> {
        self.inner_image_multi(value_id)?.get_image(index, |image| {
            let (data, size) =
                image.ok_or_else(|| PyValueError::new_err("ImageMulti index not found."))?;
            Ok((PyByteArray::new(py, data), (size[0], size[1])))
        })
    }

    #[pyo3(signature = (value_id, index, image, update))]
    fn image_multi_set(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        image: PyBuffer<u8>,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            let images = self.inner_image_multi(value_id)?;
            let image_data = pyimage::image_data(&image)?;
            images
                .set_image(index, image_data, update)
                .map_err(PyValueError::new_err)
        })
    }

    #[pyo3(signature = (value_id, index, shape, color, update))]
    fn image_multi_set_all(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        shape: [usize; 2],
        color: &Bound<'_, PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let rgba = pyimage::image_color(color)?;
        py.detach(|| {
            self.inner_image_multi(value_id)?
                .set_all_image(index, shape, rgba, update)
                .map_err(PyValueError::new_err)
        })
    }

    #[pyo3(signature = (value_id, index, image, origin, update, force=false))]
    #[allow(clippy::too_many_arguments)]
    fn image_multi_update(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        image: PyBuffer<u8>,
        origin: [u32; 2],
        update: bool,
        force: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            let images = self.inner_image_multi(value_id)?;
            let image_data = pyimage::image_data(&image)?;
            images
                .update_image(
                    index,
                    &[origin[0] as usize, origin[1] as usize],
                    image_data,
                    update,
                    force,
                )
                .map_err(PyValueError::new_err)
        })
    }

    fn image_multi_remove_index(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            self.inner_image_multi(value_id)?
                .remove_index(index, update)
                .map_err(PyValueError::new_err)
        })
    }

    fn image_multi_reset(&self, py: Python, value_id: u64, update: bool) -> PyResult<()> {
        py.detach(|| {
            self.inner_image_multi(value_id)?
                .reset_images(update)
                .map_err(PyValueError::new_err)
        })
    }

    fn image_multi_len(&self, value_id: u64) -> PyResult<usize> {
        Ok(self.inner_image_multi(value_id)?.len())
    }

    fn image_multi_contains(&self, value_id: u64, index: u32) -> PyResult<bool> {
        Ok(self.inner_image_multi(value_id)?.contains(index))
    }

    fn image_multi_indices(&self, value_id: u64) -> PyResult<Vec<u32>> {
        Ok(self.inner_image_multi(value_id)?.indices())
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
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .set(elements, update)
                .map_err(PyValueError::new_err)
        })
    }

    fn data_add(
        &self,
        py: Python,
        value_id: u64,
        data: &Bound<PyAny>,
        update: bool,
    ) -> PyResult<()> {
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .add(elements, update)
                .map_err(PyValueError::new_err)
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
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .replace(elements, index, update)
                .map_err(PyValueError::new_err)
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
                .map_err(PyValueError::new_err)
        })
    }

    fn data_clear(&self, py: Python, value_id: u64, update: bool) -> PyResult<()> {
        py.detach(|| {
            self.inner_data(value_id)?
                .clear(update)
                .map_err(PyValueError::new_err)
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
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data_take(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .set(elements, blocking, update, cache)
                .map_err(PyValueError::new_err)
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
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data_multi(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .set(index, elements, update)
                .map_err(PyValueError::new_err)
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
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data_multi(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .add(index, elements, update)
                .map_err(PyValueError::new_err)
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
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data_multi(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .replace(index, data_index, elements, update)
                .map_err(PyValueError::new_err)
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
                .map_err(PyValueError::new_err)
        })
    }

    fn data_multi_clear(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            self.inner_data_multi(value_id)?
                .clear(index, update)
                .map_err(PyValueError::new_err)
        })
    }

    fn data_multi_remove_index(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            self.inner_data_multi(value_id)?
                .remove_index(index, update)
                .map_err(PyValueError::new_err)
        })
    }

    fn data_multi_reset(&self, py: Python, value_id: u64, update: bool) -> PyResult<()> {
        py.detach(|| {
            self.inner_data_multi(value_id)?
                .reset(update)
                .map_err(PyValueError::new_err)
        })
    }

    // data multi take --------------------------------------------------
    #[allow(clippy::too_many_arguments)]
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
        let buffer = data_buffer(data)?;
        let data_value = self.inner_data_multi_take(value_id)?;
        let elements = data_slice(&buffer, data_value.data_type)?;

        py.detach(|| {
            data_value
                .set(index, elements, blocking, update, cache)
                .map_err(PyValueError::new_err)
        })
    }

    fn data_multi_take_remove_index(
        &self,
        py: Python,
        value_id: u64,
        index: u32,
        update: bool,
    ) -> PyResult<()> {
        py.detach(|| {
            self.inner_data_multi_take(value_id)?
                .remove_index(index, update)
                .map_err(PyValueError::new_err)
        })
    }

    fn data_multi_take_reset(&self, py: Python, value_id: u64, update: bool) -> PyResult<()> {
        py.detach(|| {
            self.inner_data_multi_take(value_id)?
                .reset(update)
                .map_err(PyValueError::new_err)
        })
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

        let type_id = ObjectType::map_hash(
            &key_object_type.get_core_type(py)?,
            &value_object_type.get_core_type(py)?,
        );
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

    fn add_image_multi(&self, name: String) -> PyResult<u64> {
        let value_id = self
            .server
            .write()
            .add_image_multi(&name)
            .map_err(|e| PyValueError::new_err(format!("Failed to add ImageMulti: {}", e)))?;
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

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use pyo3::types::PyBytes;
    use tokio_tungstenite::tungstenite::{self, Message};

    use super::*;
    use crate::protocol::ClientHeader;

    /// A raw client that completes the handshake and never acknowledges.
    fn silent_client(port: u16) -> tungstenite::WebSocket<TcpStream> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        let (mut socket, _) =
            tungstenite::client(format!("ws://127.0.0.1:{port}"), stream).unwrap();
        let handshake = ClientHeader::Handshake(crate::PROTOCOL_VERSION, None, None);
        let bytes = postcard::to_stdvec(&handshake).unwrap();
        socket.send(Message::Binary(bytes.into())).unwrap();
        socket
    }

    /// A started server with one `u8` DataMulti and a connected silent client.
    fn silent_session() -> (
        Arc<Py<StateServerCore>>,
        u64,
        tungstenite::WebSocket<TcpStream>,
    ) {
        Python::initialize();
        let core = Python::attach(|py| {
            let core = StateServerCore::new(None, 60.0, None, None, Vec::new()).unwrap();
            Py::new(py, core).unwrap()
        });
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let id = Python::attach(|py| {
            let server = core.borrow(py);
            let id = server.add_data_multi("root.data".to_string(), 0).unwrap();
            server.finalize(py).unwrap();
            server.start(py, port, Some([127, 0, 0, 1]), None).unwrap();
            id
        });
        let client = silent_client(port);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !Python::attach(|py| core.borrow(py).is_connected()) {
            assert!(Instant::now() < deadline, "timed out: client connection");
            thread::sleep(Duration::from_millis(10));
        }
        (Arc::new(core), id, client)
    }

    /// A control that waits behind a producer must release the GIL, or every
    /// other Python thread stalls until the client acknowledges.
    #[test]
    fn data_multi_controls_release_the_gil_while_waiting() {
        if crate::test_support::isolated() {
            return;
        }
        type Control = fn(&StateServerCore, Python, u64) -> PyResult<()>;
        let controls: [(&str, Control); 3] = [
            ("clear", |server, py, id| {
                server.data_multi_clear(py, id, 0, false)
            }),
            ("remove_index", |server, py, id| {
                server.data_multi_remove_index(py, id, 0, false)
            }),
            ("reset", |server, py, id| {
                server.data_multi_reset(py, id, false)
            }),
        ];
        for (name, control) in controls {
            let (core, id, client) = silent_session();

            // The second set waits for an acknowledgement that never comes.
            let producer = {
                let core = core.clone();
                thread::spawn(move || {
                    Python::attach(|py| {
                        let server = core.borrow(py);
                        let data = PyBytes::new(py, &[1, 2, 3]);
                        server.data_multi_set(py, id, 0, data.as_any(), false)?;
                        server.data_multi_set(py, id, 0, data.as_any(), false)
                    })
                })
            };
            thread::sleep(Duration::from_millis(300));
            let waiting = {
                let core = core.clone();
                thread::spawn(move || Python::attach(|py| control(&core.borrow(py), py, id)))
            };
            thread::sleep(Duration::from_millis(300));

            let (tx, rx) = mpsc::channel();
            thread::spawn(move || Python::attach(|_| tx.send(()).unwrap()));
            assert!(
                rx.recv_timeout(Duration::from_secs(2)).is_ok(),
                "a waiting DataMulti {name} holds the GIL"
            );

            // Closing the connection releases both waits.
            drop(client);
            producer.join().unwrap().unwrap();
            waiting.join().unwrap().unwrap();
            Python::attach(|py| core.borrow(py).stop(py));
        }
    }
}
