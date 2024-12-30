use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;
use pyo3::types::PyList;
use serde::{Deserialize, Serialize};

use crate::python_convert::ToPython;
use crate::transport::{deserealize, serialize, MessageData, WriteMessage};
use crate::SyncTrait;

#[derive(Serialize)]
enum ListMessageRef<'a, T> {
    All(&'a Vec<T>),
    Set(usize, &'a T),
    Add(&'a T),
    Remove(usize),
}

#[derive(Deserialize)]
enum ListMessage<T> {
    All(Vec<T>),
    Set(usize, T),
    Add(T),
    Remove(usize),
}

pub(crate) trait ListUpdate: Sync + Send {
    fn update_list(&self, data: MessageData) -> Result<(), String>;
}

pub struct ValueList<T> {
    id: u32,
    list: RwLock<Vec<T>>,
}

impl<T: Clone> ValueList<T> {
    pub(crate) fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            id,
            list: RwLock::new(Vec::new()),
        })
    }

    pub fn get(&self) -> Vec<T> {
        self.list.read().unwrap().clone()
    }

    pub fn get_item(&self, idx: usize) -> Option<T> {
        self.list.read().unwrap().get(idx).cloned()
    }

    pub fn process<R>(&self, op: impl Fn(&Vec<T>) -> R) -> R {
        let l = self.list.read().unwrap();
        op(&*l)
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> ListUpdate for ValueList<T> {
    fn update_list(&self, data: MessageData) -> Result<(), String> {
        let message = deserealize(data)
            .map_err(|e| format!("Error deserializing message {} with id {}", e, self.id))?;

        match message {
            ListMessage::All(list) => {
                *self.list.write().unwrap() = list;
            }
            ListMessage::Set(idx, value) => {
                let mut list = self.list.write().unwrap();
                if idx < list.len() {
                    list[idx] = value;
                }
            }
            ListMessage::Add(value) => {
                self.list.write().unwrap().push(value);
            }
            ListMessage::Remove(idx) => {
                let mut list = self.list.write().unwrap();
                if idx < list.len() {
                    list.remove(idx);
                }
            }
        }
        Ok(())
    }
}

// SERVER ---------------------------------------------------
// ----------------------------------------------------------
pub(crate) trait PyListTrait: Send + Sync {
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyList>;
    fn get_item_py<'py>(&self, py: Python<'py>, idx: usize) -> PyResult<Bound<'py, PyAny>>;
    fn set_py(&self, list: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn set_item_py(&self, idx: usize, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn add_item_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
    fn del_item_py(&self, idx: usize, update: bool) -> PyResult<()>;
    fn len_py(&self) -> usize;
}

pub struct PyValueList<T> {
    id: u32,
    list: RwLock<Vec<T>>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl<T> PyValueList<T> {
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

impl<T> PyListTrait for PyValueList<T>
where
    T: Serialize + ToPython + for<'py> FromPyObject<'py> + Clone,
{
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyList> {
        let list = self.list.read().unwrap().clone();
        let py_list = PyList::empty(py);
        for val in list.iter() {
            let val = val.to_python(py);
            py_list.append(val).unwrap();
        }
        py_list
    }

    fn get_item_py<'py>(&self, py: Python<'py>, idx: usize) -> PyResult<Bound<'py, PyAny>> {
        let list = self.list.read().unwrap();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        Ok(list[idx].to_python(py))
    }

    fn set_py(&self, list: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let list = list.downcast::<pyo3::types::PyList>()?;
        let mut data = Vec::new();
        for val in list {
            data.push(val.extract()?);
        }

        let mut l = self.list.write().unwrap();

        if self.connected.load(Ordering::Relaxed) {
            let data = serialize(ListMessageRef::All(&data));
            let message = WriteMessage::List(self.id, update, data);

            self.channel.send(message).unwrap();
        }

        *l = data;

        Ok(())
    }

    fn set_item_py(&self, idx: usize, list: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = list.extract()?;
        let mut new_list = self.list.write().unwrap();
        if idx >= new_list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        if self.connected.load(Ordering::Relaxed) {
            list.py().allow_threads(|| {
                let data = serialize(ListMessageRef::Set(idx, &value));
                let message = WriteMessage::List(self.id, update, data);
                self.channel.send(message).unwrap();
            });
        }

        new_list[idx] = value;

        Ok(())
    }

    fn del_item_py(&self, idx: usize, update: bool) -> PyResult<()> {
        let mut list = self.list.write().unwrap();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        if self.connected.load(Ordering::Relaxed) {
            let data = serialize(ListMessageRef::Remove::<T>(idx));
            let message = WriteMessage::List(self.id, update, data);
            self.channel.send(message).unwrap();
        }

        list.remove(idx);

        Ok(())
    }

    fn add_item_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = value.extract()?;

        let mut list = self.list.write().unwrap();
        if self.connected.load(Ordering::Relaxed) {
            let data = serialize(ListMessageRef::Add(&value));
            let message = WriteMessage::List(self.id, update, data);

            self.channel.send(message).unwrap();
        }

        list.push(value);

        Ok(())
    }

    fn len_py(&self) -> usize {
        self.list.read().unwrap().len()
    }
}

impl<T: Serialize + Send + Sync> SyncTrait for PyValueList<T> {
    fn sync(&self) {
        let list = self.list.read().unwrap();
        let data = serialize(ListMessageRef::All(&list));
        let message = WriteMessage::List(self.id, false, data);
        self.channel.send(message).unwrap();
    }
}
