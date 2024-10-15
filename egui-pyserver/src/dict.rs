use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;

use egui_pytransport::collections::ItemWriteRead;
use egui_pytransport::dict::DictMessage;
use egui_pytransport::transport::WriteMessage;

use crate::py_convert::FromPyValue;
use crate::SyncTrait;

pub(crate) trait PyDict: Send + Sync {
    fn get_py(&self, py: Python) -> PyObject;
    fn get_item_py(&self, key: &Bound<PyAny>) -> PyResult<PyObject>;
    fn set_py(&self, dict: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn set_item_py(&self, key: &Bound<PyAny>, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn del_item_py(&self, key: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn len_py(&self) -> usize;
}

pub struct ValueDict<K, V> {
    id: u32,
    dict: RwLock<HashMap<K, V>>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl<K, V> ValueDict<K, V> {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            dict: RwLock::new(HashMap::new()),
            channel,
            connected,
        })
    }
}

impl<K, V> PyDict for ValueDict<K, V>
where
    K: ItemWriteRead + ToPyObject + FromPyValue + Eq + Hash,
    V: ItemWriteRead + ToPyObject + FromPyValue,
{
    fn get_py(&self, py: Python) -> PyObject {
        let dict = self.dict.read().unwrap();

        let py_dict = pyo3::types::PyDict::new_bound(py);
        for (key, value) in dict.iter() {
            let key = key.to_object(py);
            let value = value.to_object(py);
            py_dict.set_item(key, value).unwrap();
        }

        py_dict.into()
    }

    fn get_item_py(&self, key: &Bound<PyAny>) -> PyResult<PyObject> {
        let dict_key = K::from_python(key)?;
        let dict = self.dict.read().unwrap();

        match dict.get(&dict_key) {
            Some(value) => Ok(value.to_object(key.py())),
            None => Err(PyKeyError::new_err("Key not found.")),
        }
    }

    fn del_item_py(&self, key: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let dict_key = K::from_python(key)?;

        let mut d = self.dict.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let message: DictMessage<K, V> = DictMessage::Remove(dict_key.clone());
            let message = WriteMessage::dict(self.id, update, message);
            self.channel.send(message).unwrap();
        }

        d.remove(&dict_key);
        Ok(())
    }

    fn set_item_py(&self, key: &Bound<PyAny>, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let dict_key = K::from_python(key)?;
        let dict_value = V::from_python(value)?;

        let mut d = self.dict.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let message: DictMessage<K, V> = DictMessage::Set(dict_key.clone(), dict_value.clone());
            let message = WriteMessage::dict(self.id, update, message);
            self.channel.send(message).unwrap();
        }

        d.insert(dict_key, dict_value);
        Ok(())
    }

    fn set_py(&self, dict: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let dict = dict.downcast::<pyo3::types::PyDict>()?;
        let mut new_dict = HashMap::new();

        for (key, value) in dict {
            let key = K::from_python(&key)?;
            let value = V::from_python(&value)?;
            new_dict.insert(key, value);
        }

        let mut d = self.dict.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let message: DictMessage<K, V> = DictMessage::All(new_dict.clone());
            let message = WriteMessage::dict(self.id, update, message);
            self.channel.send(message).unwrap();
        }

        *d = new_dict;
        Ok(())
    }

    fn len_py(&self) -> usize {
        self.dict.read().unwrap().len()
    }
}

impl<K, V> SyncTrait for ValueDict<K, V>
where
    K: ItemWriteRead,
    V: ItemWriteRead,
{
    fn sync(&self) {
        let dict = self.dict.read().unwrap().clone();
        let message = WriteMessage::dict(self.id, false, DictMessage::All(dict));
        self.channel.send(message).unwrap();
    }
}
