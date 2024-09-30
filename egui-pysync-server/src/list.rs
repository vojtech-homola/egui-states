use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;

use egui_pysync_common::collections::ItemWriteRead;
use egui_pysync_common::transport::{self, ListMessage};

use crate::py_convert::PyConvert;
use crate::transport::WriteMessage;
use crate::SyncTrait;

pub(crate) trait WriteListMessage: Send + Sync {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>>;
}

impl<T: ItemWriteRead> WriteListMessage for ListMessage<T> {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            ListMessage::All(list) => {
                head[0] = transport::LIST_ALL;

                let size = list.len() * T::size();
                head[1..9].copy_from_slice(&(list.len() as u64).to_le_bytes());
                head[9..17].copy_from_slice(&(size as u64).to_le_bytes());

                if size > 0 {
                    let mut data = vec![0; size];
                    for (i, val) in list.iter().enumerate() {
                        val.write(data[i * T::size()..].as_mut());
                    }

                    Some(data)
                } else {
                    None
                }
            }

            ListMessage::Set(idx, value) => {
                head[0] = transport::LIST_SET;

                let size = T::size();
                if size + 8 >= transport::MESS_SIZE - 2 {
                    head[1] = 0;
                    head[2..10].copy_from_slice(&(*idx as u64).to_le_bytes());
                    value.write(head[10..].as_mut());
                    return None;
                }

                head[1] = 255;
                head[2..10].copy_from_slice(&(*idx as u64).to_le_bytes());
                head[10..14].copy_from_slice(&(size as u32).to_le_bytes());
                let mut data = vec![0; size];
                value.write(data[0..].as_mut());
                Some(data)
            }

            ListMessage::Add(value) => {
                head[0] = transport::LIST_ADD;

                let size = T::size();
                if size + 8 >= transport::MESS_SIZE - 2 {
                    head[1] = 0;
                    value.write(head[2..].as_mut());
                    return None;
                }

                head[1] = 255;
                head[2..6].copy_from_slice(&(size as u32).to_le_bytes());
                let mut data = vec![0; size];
                value.write(data[0..].as_mut());
                Some(data)
            }

            ListMessage::Remove(idx) => {
                head[0] = transport::LIST_REMOVE;
                head[1..9].copy_from_slice(&(*idx as u64).to_le_bytes());
                None
            }
        }
    }
}

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
    T: ItemWriteRead + ToPyObject + PyConvert,
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
            data.push(T::from_python(&val)?);
        }

        if self.connected.load(Ordering::Relaxed) {
            let message = ListMessage::All(data.clone());
            let message = WriteMessage::list(self.id, update, message);

            self.channel.send(message).unwrap();
        }

        *self.list.write().unwrap() = data;

        Ok(())
    }

    fn set_item_py(&self, idx: usize, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let mut list = self.list.write().unwrap();
        if idx >= list.len() {
            return Err(PyIndexError::new_err("list index out of range"));
        }

        let value = T::from_python(&value)?;
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
        let value = T::from_python(&value)?;
        if self.connected.load(Ordering::Relaxed) {
            let message = ListMessage::Add(value.clone());
            let message = WriteMessage::list(self.id, update, message);

            self.channel.send(message).unwrap();
        }

        self.list.write().unwrap().push(value);

        Ok(())
    }

    fn len_py(&self) -> usize {
        self.list.read().unwrap().len()
    }
}

impl<T: ItemWriteRead> SyncTrait for ValueList<T> {
    fn sync(&self) {
        let list = self.list.read().unwrap().clone();
        let message = WriteMessage::list(self.id, false, ListMessage::All(list));
        self.channel.send(message).unwrap();
    }
}
