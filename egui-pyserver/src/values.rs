use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::conversion::IntoPyObject;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use egui_pysync::transport::WriteMessage;
use egui_pysync::values::{ReadValue, ValueMessage, WriteValue};
use egui_pysync::EnumInt;

use crate::signals::ChangedValues;
use crate::ToPython;
use crate::{Acknowledge, SyncTrait};

pub(crate) trait ProccesValue: Send + Sync {
    fn read_value(&self, head: &[u8], data: Option<Vec<u8>>, signal: bool) -> Result<(), String>;
}

pub(crate) trait PyValue: Send + Sync {
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
    fn set_py(&self, value: &Bound<PyAny>, set_signal: bool, update: bool) -> PyResult<()>;
}

pub(crate) trait PyValueStatic: Send + Sync {
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
    fn set_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()>;
}

pub(crate) trait PySignal: Send + Sync {
    fn set_py(&self, value: &Bound<PyAny>) -> PyResult<()>;
}

pub(crate) struct EnumType;
pub(crate) struct NonEnumType;

// Value ---------------------------------------------------
pub struct Value<T, M> {
    id: u32,
    value: RwLock<(T, usize)>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
    signals: ChangedValues,
    marker: PhantomData<M>,
}

impl<T, M> Value<T, M> {
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
            marker: PhantomData,
        })
    }
}

impl<T> PyValue for Value<T, NonEnumType>
where
    T: WriteValue + Clone + ToPython + for<'py> FromPyObject<'py>,
{
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value.read().unwrap().0.to_python(py)
    }

    fn set_py(&self, value: &Bound<PyAny>, set_signal: bool, update: bool) -> PyResult<()> {
        let value: T = value.extract()?;
        if self.connected.load(Ordering::Relaxed) {
            let message = WriteMessage::Value(self.id, update, value.clone().into_message());
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

impl<T> PyValue for Value<T, EnumType>
where
    T: EnumInt,
{
    fn get_py<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value
            .read()
            .unwrap()
            .0
            .as_int()
            .into_pyobject(py)
            .unwrap()
            .into_any()
    }

    fn set_py(&self, value: &Bound<PyAny>, set_signal: bool, update: bool) -> PyResult<()> {
        let int_val = value.getattr("value")?.extract::<u64>()?;
        let value =
            T::from_int(int_val).map_err(|_| PyValueError::new_err("Invalid enum value"))?;
        if self.connected.load(Ordering::Relaxed) {
            let message = WriteMessage::Value(self.id, update, ValueMessage::U64(int_val));
            let mut w = self.value.write().unwrap();
            w.0 = value.clone();
            w.1 += 1;
            self.channel.send(message).unwrap();
            if set_signal {
                self.signals.set(self.id, int_val);
            }
        } else {
            let mut w = self.value.write().unwrap();
            w.0 = value.clone();
            if set_signal {
                self.signals.set(self.id, int_val);
            }
        }

        Ok(())
    }
}

impl<T> ProccesValue for Value<T, NonEnumType>
where
    T: ReadValue + WriteValue + ToPython,
{
    fn read_value(&self, head: &[u8], data: Option<Vec<u8>>, signal: bool) -> Result<(), String> {
        let value = T::read_message(head, data)?;

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

impl<T> ProccesValue for Value<T, EnumType>
where
    T: EnumInt,
{
    fn read_value(&self, head: &[u8], data: Option<Vec<u8>>, signal: bool) -> Result<(), String> {
        let int_val = u64::read_message(head, data)?;
        let value = T::from_int(int_val).map_err(|_| "Invalid enum format".to_string())?;

        let mut w = self.value.write().unwrap();
        if w.1 == 0 {
            w.0 = value.clone();
        }

        if signal {
            self.signals.set(self.id, int_val);
        }
        Ok(())
    }
}

impl<T: Sync + Send, M: Send + Sync> Acknowledge for Value<T, M> {
    fn acknowledge(&self) {
        let mut w = self.value.write().unwrap();
        if w.1 > 0 {
            w.1 -= 1;
        }
    }
}

impl<T: Sync + Send, M: Send + Sync> SyncTrait for Value<T, M>
where
    T: WriteValue + Clone,
{
    fn sync(&self) {
        let mut w = self.value.write().unwrap();
        w.1 = 1;
        let message = w.0.clone().into_message();
        drop(w);

        let message = WriteMessage::Value(self.id, false, message);
        self.channel.send(message).unwrap();
    }
}

// ValueStatic ---------------------------------------------------
pub struct ValueStatic<T, M> {
    id: u32,
    value: RwLock<T>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
    marker: PhantomData<M>,
}

impl<T, M> ValueStatic<T, M> {
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
            marker: PhantomData,
        })
    }
}

impl<T> PyValueStatic for ValueStatic<T, NonEnumType>
where
    T: WriteValue + Clone + for<'py> FromPyObject<'py> + ToPython,
{
    fn get_py<'a, 'py>(&'a self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value.read().unwrap().to_python(py)
    }

    fn set_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let value: T = value.extract()?;
        if self.connected.load(Ordering::Relaxed) {
            let message = WriteMessage::Static(self.id, update, value.clone().into_message());
            let mut v = self.value.write().unwrap();
            *v = value;
            self.channel.send(message).unwrap();
        } else {
            *self.value.write().unwrap() = value;
        }

        Ok(())
    }
}

impl<T> PyValueStatic for ValueStatic<T, EnumType>
where
    T: EnumInt,
{
    fn get_py<'a, 'py>(&'a self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.value
            .read()
            .unwrap()
            .as_int()
            .into_pyobject(py)
            .unwrap()
            .into_any()
    }

    fn set_py(&self, value: &Bound<PyAny>, update: bool) -> PyResult<()> {
        let int_val = value.getattr("value")?.extract::<u64>()?;
        let value: T =
            T::from_int(int_val).map_err(|_| PyValueError::new_err("Invalid enum value"))?;
        if self.connected.load(Ordering::Relaxed) {
            let message = WriteMessage::Static(self.id, update, ValueMessage::U64(int_val));
            let mut v = self.value.write().unwrap();
            *v = value;
            self.channel.send(message).unwrap();
        } else {
            *self.value.write().unwrap() = value;
        }

        Ok(())
    }
}

impl<T: Sync + Send, M: Send + Sync> SyncTrait for ValueStatic<T, M>
where
    T: WriteValue + Clone,
{
    fn sync(&self) {
        let message = self.value.write().unwrap().clone().into_message();
        let message = WriteMessage::Static(self.id, false, message);
        self.channel.send(message).unwrap();
    }
}

// Signal ---------------------------------------------------
pub struct Signal<T, M> {
    id: u32,
    signals: ChangedValues,
    phantom: PhantomData<T>,
    marker: PhantomData<M>,
}

impl<T, M> Signal<T, M> {
    pub(crate) fn new(id: u32, signals: ChangedValues) -> Arc<Self> {
        Arc::new(Self {
            id,
            signals,
            phantom: PhantomData,
            marker: PhantomData,
        })
    }
}

impl<T> ProccesValue for Signal<T, NonEnumType>
where
    T: ReadValue + WriteValue + ToPython,
{
    fn read_value(&self, head: &[u8], data: Option<Vec<u8>>, _: bool) -> Result<(), String> {
        let value = T::read_message(head, data)?;
        self.signals.set(self.id, value);
        Ok(())
    }
}

impl<T> ProccesValue for Signal<T, EnumType>
where
    T: EnumInt,
{
    fn read_value(&self, head: &[u8], data: Option<Vec<u8>>, _: bool) -> Result<(), String> {
        let int_val = u64::read_message(head, data)?;
        self.signals.set(self.id, int_val);
        Ok(())
    }
}

impl<T> PySignal for Signal<T, NonEnumType>
where
    T: for<'py> FromPyObject<'py> + ToPython + Send + Sync + 'static,
{
    fn set_py(&self, value: &Bound<PyAny>) -> PyResult<()> {
        let value: T = value.extract()?;
        self.signals.set(self.id, value);
        Ok(())
    }
}

impl<T> PySignal for Signal<T, EnumType>
where
    T: EnumInt,
{
    fn set_py(&self, value: &Bound<PyAny>) -> PyResult<()> {
        let int_val = value.getattr("value")?.extract::<u64>()?;
        T::from_int(int_val).map_err(|_| PyValueError::new_err("Invalid enum value"))?;
        self.signals.set(self.id, int_val);
        Ok(())
    }
}
