use parking_lot::RwLock;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core_old::serialization::{TYPE_STATIC, TYPE_VALUE, deserialize, serialize_vec};

use crate::python_convert::{FromPython, ToPython};
use crate::sender::MessageSender;
use crate::server::{Acknowledge, SyncTrait};
use crate::signals::ChangedValues;

pub(crate) trait UpdateValueServer: Send + Sync {
    fn update_value(&self, data: &[u8]) -> Result<(), String>;
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
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    signals: ChangedValues,
}

impl<T> PyValue<T> {
    pub(crate) fn new(
        id: u32,
        value: T,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        signals: ChangedValues,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new((value, 0)),
            sender,
            connected,
            signals,
        })
    }
}

impl<T> PyValueTrait for PyValue<T>
where
    T: Serialize + Clone + ToPython + FromPython + 'static,
{
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value.read().0.to_python(py)
    }

    fn set_py(&self, value: &Bound<PyAny>, set_signal: bool, update: bool) -> PyResult<()> {
        let value: T = T::from_python(value)?;
        if self.connected.load(Ordering::Relaxed) {
            let data = serialize_vec(self.id, (update, &value), TYPE_VALUE);
            let mut w = self.value.write();
            w.0 = value.clone();
            w.1 += 1;
            self.sender.send(Bytes::from(data));
            if set_signal {
                self.signals.set(self.id, value);
            }
        } else {
            let mut w = self.value.write();
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
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let (signal, value): (bool, T) = deserialize(data)
            .map_err(|e| format!("Parse error: {} for value id: {}", e, self.id))?;

        let mut w = self.value.write();
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
        let mut w = self.value.write();
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
        let mut w = self.value.write();
        w.1 = 1;
        let data = serialize_vec(self.id, (false, &w.0), TYPE_VALUE);
        drop(w);

        self.sender.send(Bytes::from(data));
    }
}

// PyValueStatic --------------------------------------------
pub(crate) struct PyValueStatic<T> {
    id: u32,
    value: RwLock<T>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl<T> PyValueStatic<T> {
    pub(crate) fn new(
        id: u32,
        value: T,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            value: RwLock::new(value),
            sender,
            connected,
        })
    }
}

impl<T> PyValueStaticTrait for PyValueStatic<T>
where
    T: Serialize + Clone + FromPython + ToPython,
{
    fn get_py<'a, 'py>(&'a self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value.read().to_python(py)
    }

    fn set_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = T::from_python(value)?;
        if self.connected.load(Ordering::Relaxed) {
            let data = serialize_vec(self.id, (update, &value), TYPE_STATIC);
            let mut v = self.value.write();
            *v = value;
            self.sender.send(Bytes::from(data));
        } else {
            *self.value.write() = value;
        }

        Ok(())
    }
}

impl<T: Sync + Send> SyncTrait for PyValueStatic<T>
where
    T: Serialize + Clone,
{
    fn sync(&self) {
        let w = self.value.read();
        let data = serialize_vec(self.id, (false, &(*w)), TYPE_STATIC);
        self.sender.send(Bytes::from(data));
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
    fn update_value(&self, data: &[u8]) -> Result<(), String> {
        let value: T = deserialize(data)
            .map_err(|e| format!("Parse error: {} for signal id: {}", e, self.id))?;
        self.signals.set(self.id, value);
        Ok(())
    }
}

impl<T> PySignalTrait for PySignal<T>
where
    T: FromPython + ToPython + Send + Sync + 'static,
{
    fn set_py(&self, value: &Bound<PyAny>) -> PyResult<()> {
        let value: T = T::from_python(value)?;
        self.signals.set(self.id, value);
        Ok(())
    }
}
