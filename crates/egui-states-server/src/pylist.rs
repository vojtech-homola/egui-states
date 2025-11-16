use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;
use pyo3::types::PyList;
use serde::Serialize;
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::serialization::{TYPE_LIST, serialize_vec};

use crate::python_convert::{FromPython, ToPython};
use crate::sender::MessageSender;
use crate::server::SyncTrait;

#[derive(Serialize)]
enum ListMessageRef<'a, T> {
    All(&'a Vec<T>),
    Set(usize, &'a T),
    Add(&'a T),
    Remove(usize),
}

pub(crate) trait PyListTrait: Send + Sync {
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyList>;
    fn get_item_py<'py>(&self, py: Python<'py>, idx: usize) -> PyResult<Bound<'py, PyAny>>;
    fn set_py(&self, list: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn set_item_py(&self, idx: usize, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn add_item_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn del_item_py(&self, idx: usize, update: bool) -> PyResult<()>;
    fn len_py(&self) -> usize;
}

pub(crate) struct PyValueList<T> {
    id: u32,
    list: RwLock<Vec<T>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl<T> PyValueList<T> {
    pub(crate) fn new(id: u32, sender: MessageSender, connected: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            id,
            list: RwLock::new(Vec::new()),
            sender,
            connected,
        })
    }
}

impl<T> PyListTrait for PyValueList<T>
where
    T: Serialize + ToPython + FromPython + Clone,
{
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyList> {
        let list = self.list.read().clone();
        let py_list = PyList::empty(py);
        for val in list.iter() {
            let val = val.to_python(py);
            py_list.append(val).unwrap();
        }
        py_list
    }

    fn get_item_py<'py>(&self, py: Python<'py>, idx: usize) -> PyResult<Bound<'py, PyAny>> {
        let list = self.list.read();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        Ok(list[idx].to_python(py))
    }

    fn set_py(&self, list: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let list = list.cast::<pyo3::types::PyList>()?;
        let mut data = Vec::new();
        for val in list {
            data.push(T::from_python(&val)?);
        }

        let mut l = self.list.write();

        if self.connected.load(Ordering::Relaxed) {
            let data = serialize_vec(self.id, (update, ListMessageRef::All(&data)), TYPE_LIST);
            self.sender.send(Bytes::from(data));
        }

        *l = data;

        Ok(())
    }

    fn set_item_py(&self, idx: usize, list: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = T::from_python(list)?;
        let mut new_list = self.list.write();
        if idx >= new_list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        if self.connected.load(Ordering::Relaxed) {
            list.py().detach(|| {
                let data = serialize_vec(
                    self.id,
                    (update, ListMessageRef::Set(idx, &value)),
                    TYPE_LIST,
                );
                self.sender.send(Bytes::from(data));
            });
        }

        new_list[idx] = value;

        Ok(())
    }

    fn del_item_py(&self, idx: usize, update: bool) -> PyResult<()> {
        let mut list = self.list.write();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        if self.connected.load(Ordering::Relaxed) {
            let data = serialize_vec(
                self.id,
                (update, ListMessageRef::Remove::<T>(idx)),
                TYPE_LIST,
            );
            self.sender.send(Bytes::from(data));
        }

        list.remove(idx);

        Ok(())
    }

    fn add_item_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = T::from_python(value)?;

        let mut list = self.list.write();
        if self.connected.load(Ordering::Relaxed) {
            let data = serialize_vec(self.id, (update, ListMessageRef::Add(&value)), TYPE_LIST);
            self.sender.send(Bytes::from(data));
        }

        list.push(value);

        Ok(())
    }

    fn len_py(&self) -> usize {
        self.list.read().len()
    }
}

impl<T: Serialize + Send + Sync + Clone> SyncTrait for PyValueList<T> {
    fn sync(&self) {
        let list = self.list.read().clone();
        let data = serialize_vec(self.id, (false, ListMessageRef::All(&list)), TYPE_LIST);
        self.sender.send(Bytes::from(data));
    }
}
