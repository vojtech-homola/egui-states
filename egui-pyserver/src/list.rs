use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;

use egui_pysync::collections::CollectionItem;
use egui_pysync::list::ListMessage;
use egui_pysync::transport::WriteMessage;

use crate::SyncTrait;

pub(crate) trait PyListTrait: Send + Sync {
    fn get_py(&self, py: Python) -> PyObject;
    fn get_item_py(&self, py: Python, idx: usize) -> PyResult<PyObject>;
    fn set_py(&self, list: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn set_item_py(&self, idx: usize, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn add_item_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn del_item_py(&self, idx: usize, update: bool) -> PyResult<()>;
    fn len_py(&self) -> usize;
}

pub struct ValueList<T> {
    id: u32,
    list: RwLock<Vec<T>>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl<T> ValueList<T> {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            list: RwLock::new(Vec::new()),
            channel,
            connected,
        })
    }
}

impl<T> PyListTrait for ValueList<T>
where
    T: CollectionItem + ToPyObject + for<'py> FromPyObject<'py>,
{
    fn get_py(&self, py: Python) -> PyObject {
        let list = self.list.read().unwrap().to_object(py);
        list.into()
    }

    fn get_item_py(&self, py: Python, idx: usize) -> PyResult<PyObject> {
        let list = self.list.read().unwrap();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        Ok(list[idx].to_object(py))
    }

    fn set_py(&self, list: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let list = list.downcast::<pyo3::types::PyList>()?;
        let mut data = Vec::new();
        for val in list {
            data.push(val.extract()?);
        }

        let mut l = self.list.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let message = ListMessage::All(data.clone());
            let message = WriteMessage::list(self.id, update, message);

            self.channel.send(message).unwrap();
        }

        *l = data;

        Ok(())
    }

    fn set_item_py(&self, idx: usize, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = value.extract()?;
        let mut list = self.list.write().unwrap();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        if self.connected.load(Ordering::Relaxed) {
            let message = ListMessage::Set(idx, value.clone());
            let message = WriteMessage::list(self.id, update, message);
            self.channel.send(message).unwrap();
        }

        list[idx] = value;

        Ok(())
    }

    fn del_item_py(&self, idx: usize, update: bool) -> PyResult<()> {
        let mut list = self.list.write().unwrap();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        if self.connected.load(Ordering::Relaxed) {
            let message = ListMessage::Remove::<T>(idx);
            let message = WriteMessage::list(self.id, update, message);
            self.channel.send(message).unwrap();
        }

        list.remove(idx);

        Ok(())
    }

    fn add_item_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = value.extract()?;

        let mut list = self.list.write().unwrap();
        if self.connected.load(Ordering::Relaxed) {
            let message = ListMessage::Add(value.clone());
            let message = WriteMessage::list(self.id, update, message);

            self.channel.send(message).unwrap();
        }

        list.push(value);

        Ok(())
    }

    fn len_py(&self) -> usize {
        self.list.read().unwrap().len()
    }
}

impl<T: CollectionItem> SyncTrait for ValueList<T> {
    fn sync(&self) {
        let list = self.list.read().unwrap().clone();
        let message = WriteMessage::list(self.id, false, ListMessage::All(list));
        self.channel.send(message).unwrap();
    }
}
