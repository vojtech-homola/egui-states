use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde::Serialize;
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core_2::serialization::{TYPE_DICT, serialize_vec};

use crate::FromPython;
use crate::python_convert::ToPython;
use crate::sender::MessageSender;
use crate::server::SyncTrait;

#[derive(Serialize)]
enum DictMessageRef<'a, K, V>
where
    K: Eq + Hash,
{
    All(&'a HashMap<K, V>),
    Set(&'a K, &'a V),
    Remove(&'a K),
}

pub(crate) trait PyDictTrait: Send + Sync {
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyDict>;
    fn get_item_py<'py>(&self, key: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>>;
    fn set_py(&self, dict: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn set_item_py(&self, key: &Bound<PyAny>, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn del_item_py(&self, key: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn len_py(&self) -> usize;
}

pub(crate) struct PyValueDict<K, V> {
    id: u32,
    dict: RwLock<HashMap<K, V>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl<K, V> PyValueDict<K, V> {
    pub(crate) fn new(id: u32, sender: MessageSender, connected: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            id,
            dict: RwLock::new(HashMap::new()),
            sender,
            connected,
        })
    }
}

impl<K, V> PyDictTrait for PyValueDict<K, V>
where
    K: Serialize + ToPython + FromPython + Eq + Hash,
    V: Serialize + ToPython + FromPython,
{
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyDict> {
        let dict = self.dict.read();

        let py_dict = pyo3::types::PyDict::new(py);
        for (key, value) in dict.iter() {
            let key = key.to_python(py);
            let value = value.to_python(py);
            py_dict.set_item(key, value).unwrap();
        }
        py_dict
    }

    fn get_item_py<'py>(&self, key: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let dict_key = K::from_python(key)?;
        let dict = self.dict.read();

        match dict.get(&dict_key) {
            Some(value) => Ok(value.to_python(key.py())),
            None => Err(PyKeyError::new_err("Key not found.")),
        }
    }

    fn del_item_py(&self, key: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let dict_key = K::from_python(key)?;

        let mut d = self.dict.write();
        if self.connected.load(Ordering::Relaxed) {
            let to_send = (update, DictMessageRef::Remove::<K, V>(&dict_key));
            let data = serialize_vec(self.id, to_send, TYPE_DICT);
            self.sender.send(Bytes::from(data));
        }
        d.remove(&dict_key);

        Ok(())
    }

    fn set_item_py(&self, key: &Bound<PyAny>, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let dict_key = K::from_python(key)?;
        let dict_value = V::from_python(value)?;

        let mut d = self.dict.write();

        if self.connected.load(Ordering::Relaxed) {
            let to_send = (update, DictMessageRef::Set::<K, V>(&dict_key, &dict_value));
            let data = serialize_vec(self.id, to_send, TYPE_DICT);
            self.sender.send(Bytes::from(data));
        }

        d.insert(dict_key, dict_value);
        Ok(())
    }

    fn set_py(&self, dict: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let dict = dict.cast::<pyo3::types::PyDict>()?;
        let mut new_dict = HashMap::new();

        for (key, value) in dict {
            let key = K::from_python(&key)?;
            let value = V::from_python(&value)?;
            new_dict.insert(key, value);
        }

        let mut d = self.dict.write();

        if self.connected.load(Ordering::Relaxed) {
            dict.py().detach(|| {
                let to_send = (update, DictMessageRef::All(&new_dict));
                let data = serialize_vec(self.id, to_send, TYPE_DICT);
                self.sender.send(Bytes::from(data));
            });
        }

        *d = new_dict;

        Ok(())
    }

    fn len_py(&self) -> usize {
        self.dict.read().len()
    }
}

impl<K, V> SyncTrait for PyValueDict<K, V>
where
    K: Serialize + Send + Sync + Eq + Hash,
    V: Serialize + Send + Sync,
{
    fn sync(&self) {
        let dict = self.dict.read();
        let to_send = DictMessageRef::All(&dict);
        let data = serialize_vec(self.id, to_send, TYPE_DICT);
        self.sender.send(Bytes::from(data));
    }
}
