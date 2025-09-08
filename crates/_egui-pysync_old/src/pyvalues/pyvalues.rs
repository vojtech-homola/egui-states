use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::prelude::*;

use crate::server::python_convert::ToPython;
use crate::server::server::{Acknowledge, SyncTrait};
use crate::server::signals::ChangedValues;
use crate::transport::MessageData;
use crate::transport::{WriteMessage, deserialize, serialize};

pub(crate) trait UpdateValueServer: Send + Sync {
    fn update_value(&self, data: MessageData, signal: bool) -> Result<(), String>;
}

pub(crate) trait PyValueTrait: Send + Sync {
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
    fn set_py(&self, value: &Bound<PyAny>, set_signal: bool, update: bool) -> PyResult<()>;
}

pub(crate) trait PyValueStaticTrait: Send + Sync {
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
    fn set_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
}

pub(crate) trait PySignalTrait: Send + Sync {
    fn set_py(&self, value: &Bound<PyAny>) -> PyResult<()>;
}

// PyValue --------------------------------------------------
pub(crate) struct PyValue<T> {
    id: u32,
    value: RwLock<(T, usize)>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
    signals: ChangedValues,
}

impl<T> PyValue<T> {
    pub(crate) fn new(
        id: u32,
        value: T,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
        signals: ChangedValues,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new((value, 0)),
            channel,
            connected,
            signals,
        })
    }
}

impl<T> PyValueTrait for PyValue<T>
where
    T: Serialize + Clone + ToPython + for<'py> FromPyObject<'py> + 'static,
{
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value.read().unwrap().0.to_python(py)
    }

    fn set_py(&self, value: &Bound<PyAny>, set_signal: bool, update: bool) -> PyResult<()> {
        let value: T = value.extract()?;
        if self.connected.load(Ordering::Relaxed) {
            let data = serialize(&value);
            let message = WriteMessage::Value(self.id, update, data);
            let mut w = self.value.write().unwrap();
            w.0 = value.clone();
            w.1 += 1;
            self.channel.send(message).unwrap();
            if set_signal {
                self.signals.set(self.id, value);
            }
        } else {
            let mut w = self.value.write().unwrap();
            w.0 = value.clone();
            if set_signal {
                self.signals.set(self.id, value);
            }
        }

        Ok(())
    }
}

impl<T> UpdateValueServer for PyValue<T>
where
    T: ToPython + for<'a> Deserialize<'a> + Clone + 'static,
{
    fn update_value(&self, data: MessageData, signal: bool) -> Result<(), String> {
        let value: T = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        let mut w = self.value.write().unwrap();
        if w.1 == 0 {
            w.0 = value.clone();
        }

        if signal {
            self.signals.set(self.id, value);
        }
        Ok(())
    }
}

impl<T: Sync + Send> Acknowledge for PyValue<T> {
    fn acknowledge(&self) {
        let mut w = self.value.write().unwrap();
        if w.1 > 0 {
            w.1 -= 1;
        }
    }
}

impl<T: Sync + Send> SyncTrait for PyValue<T>
where
    T: Serialize + Clone,
{
    fn sync(&self) {
        let mut w = self.value.write().unwrap();
        w.1 = 1;
        let data = serialize(&w.0);
        drop(w);

        let message = WriteMessage::Value(self.id, false, data);
        self.channel.send(message).unwrap();
    }
}

// PyValueStatic --------------------------------------------
pub(crate) struct PyValueStatic<T> {
    id: u32,
    value: RwLock<T>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl<T> PyValueStatic<T> {
    pub(crate) fn new(
        id: u32,
        value: T,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            channel,
            connected,
        })
    }
}

impl<T> PyValueStaticTrait for PyValueStatic<T>
where
    T: Serialize + Clone + for<'py> FromPyObject<'py> + ToPython,
{
    fn get_py<'a, 'py>(&'a self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value.read().unwrap().to_python(py)
    }

    fn set_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = value.extract()?;
        if self.connected.load(Ordering::Relaxed) {
            let data = serialize(&value);
            let message = WriteMessage::Static(self.id, update, data);
            let mut v = self.value.write().unwrap();
            *v = value;
            self.channel.send(message).unwrap();
        } else {
            *self.value.write().unwrap() = value;
        }

        Ok(())
    }
}

impl<T: Sync + Send> SyncTrait for PyValueStatic<T>
where
    T: Serialize + Clone,
{
    fn sync(&self) {
        let w = self.value.read().unwrap();
        let data = serialize(&(*w));
        let message = WriteMessage::Static(self.id, false, data);
        self.channel.send(message).unwrap();
    }
}

// PySignal --------------------------------------------------
pub(crate) struct PySignal<T> {
    id: u32,
    signals: ChangedValues,
    phantom: PhantomData<T>,
}

impl<T> PySignal<T> {
    pub(crate) fn new(id: u32, signals: ChangedValues) -> Arc<Self> {
        Arc::new(Self {
            id,
            signals,
            phantom: PhantomData,
        })
    }
}

impl<T> UpdateValueServer for PySignal<T>
where
    T: for<'a> Deserialize<'a> + ToPython + 'static,
{
    fn update_value(&self, data: MessageData, _: bool) -> Result<(), String> {
        let value: T = deserialize(data)
            .map_err(|e| format!("Parse error: {} for signal id: {}", e, self.id))?;
        self.signals.set(self.id, value);
        Ok(())
    }
}

impl<T> PySignalTrait for PySignal<T>
where
    T: for<'py> FromPyObject<'py> + ToPython + Send + Sync + 'static,
{
    fn set_py(&self, value: &Bound<PyAny>) -> PyResult<()> {
        let value: T = value.extract()?;
        self.signals.set(self.id, value);
        Ok(())
    }
}
