use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::client::data::private;
use crate::client::event::EventUniversal;
use crate::client::messages::{ChannelMessage, MessageSender};
use crate::data_transport::DataType;
use crate::hashing::NoHashMap;

pub(crate) enum DataTakeMessage {
    All(DataType, u64, Bytes),
    BatchStart(u64, Bytes),
    Batch(Bytes),
    BatchEnd(DataType, u64, Bytes),
}

pub(crate) enum DataMultiTakeMessage {
    Remove(u32),
    Modify(u32, DataTakeMessage, bool),
    Reset,
}

// DataTake --------------------------------------------------------------------
pub(crate) trait UpdateDataTake: Sync + Send {
    fn update(&self, message: DataTakeMessage, blocking: bool) -> Result<(), String>;
}

pub struct DataTake<T> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    inner: Arc<RwLock<Option<(Vec<T>, bool)>>>,
    buffer: Arc<Mutex<Option<Vec<T>>>>,
    sender: MessageSender,
    event: EventUniversal,
}

#[allow(private_bounds)]
impl<T> DataTake<T>
where
    T: private::GetDataType,
{
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            data_type: T::get_type(),
            element_size: T::get_type().item_size(),
            inner: Arc::new(RwLock::new(None)),
            buffer: Arc::new(Mutex::new(None)),
            sender,
            event: EventUniversal::new(),
        }
    }

    pub fn take(&self) -> Option<Vec<T>> {
        let inner = self.inner.write().take();
        if let Some((val, blocking)) = inner {
            if blocking {
                self.sender.send(ChannelMessage::Ack(self.id));
            }
            return Some(val);
        }
        None
    }

    pub fn is_some(&self) -> bool {
        self.inner.read().is_some()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_take(&self) -> Vec<T> {
        loop {
            if let Some((value, blocking)) = self.inner.write().take() {
                if blocking {
                    self.sender.send(ChannelMessage::Ack(self.id));
                }
                return value;
            }
            self.event.wait_clear_blocking();
        }
    }

    pub async fn wait_take_async(&self) -> Vec<T> {
        loop {
            if let Some((value, blocking)) = self.inner.write().take() {
                if blocking {
                    self.sender.send(ChannelMessage::Ack(self.id));
                }
                return value;
            }
            self.event.wait_clear().await;
        }
    }
}

impl<T> Clone for DataTake<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            inner: self.inner.clone(),
            buffer: self.buffer.clone(),
            sender: self.sender.clone(),
            event: self.event.clone(),
        }
    }
}

#[allow(private_bounds)]
impl<T> UpdateDataTake for DataTake<T>
where
    T: private::GetDataType + Send + Sync,
{
    fn update(&self, message: DataTakeMessage, blocking: bool) -> Result<(), String> {
        match message {
            DataTakeMessage::All(data_type, count, data) => {
                if data_type != self.data_type {
                    return Err(format!(
                        "Data type {:?} does not match expected type {:?} for DataTake: {}",
                        data_type, self.data_type, self.name
                    ));
                }
                if data.len() as u64 != count * self.element_size as u64 {
                    return Err(format!(
                        "Data size {} does not match expected count {} for DataTake: {}",
                        data.len(),
                        count,
                        self.name
                    ));
                }
                let mut buffer = Vec::with_capacity(count as usize);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr(),
                        buffer.as_mut_ptr() as *mut u8,
                        data.len(),
                    );
                    buffer.set_len(count as usize);
                }
                *self.inner.write() = Some((buffer, blocking));
                self.event.set();
                Ok(())
            }
            DataTakeMessage::BatchStart(count, data) => {
                let all_data_size = count * self.element_size as u64;
                if data.len() as u64 > all_data_size {
                    return Err(format!(
                        "Batch start data size {} exceeds total data size {}",
                        data.len(),
                        all_data_size
                    ));
                }
                if data.len() % self.element_size != 0 {
                    return Err(format!(
                        "Batch start data size {} is not a multiple of element size {}",
                        data.len(),
                        self.element_size
                    ));
                }

                let mut buffer = Vec::with_capacity(count as usize);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr(),
                        buffer.as_mut_ptr() as *mut u8,
                        data.len(),
                    );
                    buffer.set_len(data.len() / self.element_size);
                }
                self.buffer.lock().replace(buffer);
                Ok(())
            }
            DataTakeMessage::Batch(data) => match *self.buffer.lock() {
                Some(ref mut buffer) => {
                    if data.len() % self.element_size != 0 {
                        return Err(format!(
                            "Batch data size {} is not a multiple of element size {}",
                            data.len(),
                            self.element_size
                        ));
                    }
                    let count = data.len() / self.element_size;

                    if buffer.len() + count > buffer.capacity() {
                        return Err(format!(
                            "Batch data size {} exceeds total data size {}",
                            buffer.len() + count,
                            buffer.capacity()
                        ));
                    }

                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            data.as_ptr(),
                            buffer.as_mut_ptr().add(buffer.len()) as *mut u8,
                            data.len(),
                        );
                        buffer.set_len(buffer.len() + count);
                    }

                    Ok(())
                }
                None => Err(format!(
                    "No header found for DataTake: {} when updating batch",
                    self.name
                )),
            },
            DataTakeMessage::BatchEnd(data_type, count, data) => {
                if data_type != self.data_type {
                    return Err(format!(
                        "Data type {:?} does not match expected type {:?} for DataTake: {}",
                        data_type, self.data_type, self.name
                    ));
                }
                match self.buffer.lock().take() {
                    Some(mut buffer) => {
                        if data.len() % self.element_size != 0 {
                            return Err(format!(
                                "Batch data size {} is not a multiple of element size {}",
                                data.len(),
                                self.element_size
                            ));
                        }
                        let count_add = data.len() / self.element_size;

                        if buffer.len() + count_add != count as usize {
                            return Err(format!(
                                "Batch end data size {} does not match total data size {}",
                                buffer.len() + count_add,
                                count
                            ));
                        }

                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                data.as_ptr(),
                                buffer.as_mut_ptr().add(buffer.len()) as *mut u8,
                                data.len(),
                            );
                            buffer.set_len(buffer.len() + count_add);
                        }

                        *self.inner.write() = Some((buffer, blocking));
                        self.event.set();
                        Ok(())
                    }
                    None => Err(format!(
                        "No header found for DataTake: {} when updating batch end",
                        self.name
                    )),
                }
            }
        }
    }
}

// DataMultiTake ----------------------------------------------------------------
pub(crate) trait UpdateDataMultiTake: Sync + Send {
    fn update(&self, key: u32, message: DataTakeMessage, blocking: bool) -> Result<(), String>;
    fn remove(&self, key: u32);
    fn reset(&self);
}

pub struct DataMultiTake<T> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    inner: Arc<RwLock<NoHashMap<u32, (Vec<T>, bool)>>>,
    buffers: Arc<Mutex<NoHashMap<u32, Vec<T>>>>,
    sender: MessageSender,
    event: EventUniversal,
}

#[allow(private_bounds)]
impl<T> DataMultiTake<T>
where
    T: private::GetDataType,
{
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            data_type: T::get_type(),
            element_size: T::get_type().item_size(),
            inner: Arc::new(RwLock::new(NoHashMap::default())),
            buffers: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
            event: EventUniversal::new(),
        }
    }

    pub fn take(&self, key: u32) -> Option<Vec<T>> {
        let inner = self.inner.write().remove(&key);
        if let Some((val, blocking)) = inner {
            if blocking {
                self.sender.send(ChannelMessage::Ack(self.id));
            }
            return Some(val);
        }
        None
    }

    pub fn is_some(&self, key: u32) -> bool {
        self.inner.read().contains_key(&key)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_take(&self, key: u32) -> Vec<T> {
        loop {
            if let Some((value, blocking)) = self.inner.write().remove(&key) {
                if blocking {
                    self.sender.send(ChannelMessage::Ack(self.id));
                }
                return value;
            }
            self.event.wait_clear_blocking();
        }
    }

    pub async fn wait_take_async(&self, key: u32) -> Vec<T> {
        loop {
            if let Some((value, blocking)) = self.inner.write().remove(&key) {
                if blocking {
                    self.sender.send(ChannelMessage::Ack(self.id));
                }
                return value;
            }
            self.event.wait_clear().await;
        }
    }
}

impl<T> Clone for DataMultiTake<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            inner: self.inner.clone(),
            buffers: self.buffers.clone(),
            sender: self.sender.clone(),
            event: self.event.clone(),
        }
    }
}

#[allow(private_bounds)]
impl<T> UpdateDataMultiTake for DataMultiTake<T>
where
    T: private::GetDataType + Send + Sync,
{
    fn update(&self, key: u32, message: DataTakeMessage, blocking: bool) -> Result<(), String> {
        match message {
            DataTakeMessage::All(data_type, count, data) => {
                if data_type != self.data_type {
                    return Err(format!(
                        "Data type {:?} does not match expected type {:?} for DataMultiTake: {}",
                        data_type, self.data_type, self.name
                    ));
                }
                if data.len() as u64 != count * self.element_size as u64 {
                    return Err(format!(
                        "Data size {} does not match expected count {} for DataMultiTake: {}",
                        data.len(),
                        count,
                        self.name
                    ));
                }
                let mut buffer = Vec::with_capacity(count as usize);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr(),
                        buffer.as_mut_ptr() as *mut u8,
                        data.len(),
                    );
                    buffer.set_len(count as usize);
                }
                self.inner.write().insert(key, (buffer, blocking));
                self.event.set();
                Ok(())
            }
            DataTakeMessage::BatchStart(count, data) => {
                let all_data_size = count * self.element_size as u64;
                if data.len() as u64 > all_data_size {
                    return Err(format!(
                        "Batch start data size {} exceeds total data size {}",
                        data.len(),
                        all_data_size
                    ));
                }
                if data.len() % self.element_size != 0 {
                    return Err(format!(
                        "Batch start data size {} is not a multiple of element size {}",
                        data.len(),
                        self.element_size
                    ));
                }

                let mut buffer = Vec::with_capacity(count as usize);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr(),
                        buffer.as_mut_ptr() as *mut u8,
                        data.len(),
                    );
                    buffer.set_len(data.len() / self.element_size);
                }
                self.buffers.lock().insert(key, buffer);
                Ok(())
            }
            DataTakeMessage::Batch(data) => match self.buffers.lock().get_mut(&key) {
                Some(ref mut buffer) => {
                    if data.len() % self.element_size != 0 {
                        return Err(format!(
                            "Batch data size {} is not a multiple of element size {}",
                            data.len(),
                            self.element_size
                        ));
                    }
                    let count = data.len() / self.element_size;

                    if buffer.len() + count > buffer.capacity() {
                        return Err(format!(
                            "Batch data size {} exceeds total data size {}",
                            buffer.len() + count,
                            buffer.capacity()
                        ));
                    }

                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            data.as_ptr(),
                            buffer.as_mut_ptr().add(buffer.len()) as *mut u8,
                            data.len(),
                        );
                        buffer.set_len(buffer.len() + count);
                    }

                    Ok(())
                }
                None => Err(format!(
                    "No header found for DataMultiTake: {} key {} when updating batch",
                    self.name, key
                )),
            },
            DataTakeMessage::BatchEnd(data_type, count, data) => {
                if data_type != self.data_type {
                    return Err(format!(
                        "Data type {:?} does not match expected type {:?} for DataMultiTake: {}",
                        data_type, self.data_type, self.name
                    ));
                }
                match self.buffers.lock().remove(&key) {
                    Some(mut buffer) => {
                        if data.len() % self.element_size != 0 {
                            return Err(format!(
                                "Batch data size {} is not a multiple of element size {}",
                                data.len(),
                                self.element_size
                            ));
                        }
                        let count_add = data.len() / self.element_size;

                        if buffer.len() + count_add != count as usize {
                            return Err(format!(
                                "Batch end data size {} does not match total data size {}",
                                buffer.len() + count_add,
                                count
                            ));
                        }

                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                data.as_ptr(),
                                buffer.as_mut_ptr().add(buffer.len()) as *mut u8,
                                data.len(),
                            );
                            buffer.set_len(buffer.len() + count_add);
                        }

                        self.inner.write().insert(key, (buffer, blocking));
                        self.event.set();
                        Ok(())
                    }
                    None => Err(format!(
                        "No header found for DataMultiTake: {} key {} when updating batch end",
                        self.name, key
                    )),
                }
            }
        }
    }

    fn remove(&self, key: u32) {
        self.inner.write().remove(&key);
        self.buffers.lock().remove(&key);
    }

    fn reset(&self) {
        self.inner.write().clear();
        self.buffers.lock().clear();
    }
}
