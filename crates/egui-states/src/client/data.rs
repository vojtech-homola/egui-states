use std::collections::hash_map::Entry;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::client::messages::{ChannelMessage, MessageSender};
use crate::data_transport::{DataType, TransportType};
use crate::hashing::NoHashMap;

pub(crate) enum DataMessage {
    All(DataType, TransportType, Bytes),
    BatchStart(u64, Bytes),
    Batch(Bytes),
    BatchEnd(DataType, TransportType, Bytes),
    Drain(u64, u64),
    Clear,
}

pub(crate) enum DataMultiMessage {
    Remove(u32),
    Modify(u32, DataMessage),
    Reset,
}

pub(crate) mod private {
    use super::DataType;

    pub(crate) unsafe trait GetDataType: Clone + Copy {
        fn get_type() -> DataType;
    }

    macro_rules! impl_get_data_type {
        ($ty:ty, $variant:expr) => {
            unsafe impl GetDataType for $ty {
                fn get_type() -> DataType {
                    $variant
                }
            }
        };
    }

    impl_get_data_type!(u8, DataType::U8);
    impl_get_data_type!(u16, DataType::U16);
    impl_get_data_type!(u32, DataType::U32);
    impl_get_data_type!(u64, DataType::U64);
    impl_get_data_type!(i8, DataType::I8);
    impl_get_data_type!(i16, DataType::I16);
    impl_get_data_type!(i32, DataType::I32);
    impl_get_data_type!(i64, DataType::I64);
    impl_get_data_type!(f32, DataType::F32);
    impl_get_data_type!(f64, DataType::F64);
}

// Data -------------------------------------------------------------------
pub(crate) trait UpdateData: Sync + Send {
    fn update_data(&self, message: DataMessage) -> Result<(), String>;
}

pub struct Data<T> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    inner: Arc<RwLock<Vec<T>>>,
    buffer: Arc<Mutex<Option<Vec<T>>>>,
    sender: MessageSender,
}

#[allow(private_bounds)]
impl<T> Data<T>
where
    T: private::GetDataType,
{
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            data_type: T::get_type(),
            element_size: T::get_type().element_size(),
            inner: Arc::new(RwLock::new(Vec::new())),
            buffer: Arc::new(Mutex::new(None)),
            sender,
        }
    }

    pub fn get(&self) -> Vec<T> {
        let inner = self.inner.read();
        inner.clone()
    }

    pub fn read<R>(&self, f: impl Fn(&[T]) -> R) -> R {
        let inner = self.inner.read();
        f(&inner)
    }

    fn set_all(&self, data: &[u8], transport_type: TransportType) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));

        if data.len() % self.element_size != 0 {
            return Err(format!(
                "Data size {} is not a multiple of element size {}",
                data.len(),
                self.element_size
            ));
        }

        let count = data.len() / self.element_size;
        let mut buffer = Vec::with_capacity(count);
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                buffer.as_mut_ptr() as *mut u8,
                data.len(),
            );
            buffer.set_len(count);
        }
        self.save_data(buffer, transport_type)
    }

    fn batch_start(&self, data: &[u8], elements_count: u64) -> Result<(), String> {
        let all_data_size = elements_count * self.element_size as u64;
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

        let mut buffer = Vec::with_capacity(elements_count as usize);
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

    fn batch(&self, data: &[u8]) -> Result<(), String> {
        match *self.buffer.lock() {
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
                "No header found for Data: {} when updating batch",
                self.name
            )),
        }
    }

    fn batch_end(&self, data: &[u8], transport_type: TransportType) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));

        match self.buffer.lock().take() {
            Some(mut buffer) => {
                if data.len() % self.element_size != 0 {
                    return Err(format!(
                        "Batch data size {} is not a multiple of element size {}",
                        data.len(),
                        self.element_size
                    ));
                }
                let count = data.len() / self.element_size;

                if buffer.len() + count != buffer.capacity() {
                    return Err(format!(
                        "Batch end data size {} does not match total data size {}",
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

                self.save_data(buffer, transport_type)
            }
            None => Err(format!(
                "No header found for Data: {} when updating batch end",
                self.name
            )),
        }
    }

    fn drain(&self, index: u64, count: u64) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));

        let mut w = self.inner.write();
        if index as usize + count as usize > w.len() {
            return Err(format!(
                "Drain range ({} to {}) exceeds current data size {}",
                index,
                index + count,
                w.len()
            ));
        }
        w.drain(index as usize..(index as usize + count as usize));

        Ok(())
    }

    fn save_data(&self, data: Vec<T>, transport_type: TransportType) -> Result<(), String> {
        match transport_type {
            TransportType::Set(count) => {
                if data.len() as u64 != count {
                    return Err(format!(
                        "Data size {} does not match expected count {} for Set transport type",
                        data.len(),
                        count
                    ));
                }
                *self.inner.write() = data;
            }
            TransportType::Add(count) => {
                if data.len() as u64 != count {
                    return Err(format!(
                        "Data size {} does not match expected count {} for Add transport type",
                        data.len(),
                        count
                    ));
                }
                let mut w = self.inner.write();
                w.extend(data);
            }
            TransportType::Replace(start, count) => {
                if data.len() as u64 != count {
                    return Err(format!(
                        "Data size {} does not match expected count {} for Replace transport type",
                        data.len(),
                        count
                    ));
                }
                let mut w = self.inner.write();
                if start as usize + data.len() > w.len() {
                    return Err(format!(
                        "Replace range ({} to {}) exceeds current data size {}",
                        start,
                        start + count,
                        w.len()
                    ));
                }
                w.splice(start as usize..(start as usize + data.len()), data);
            }
        }
        Ok(())
    }
}

impl<T: Sync + Send> UpdateData for Data<T>
where
    T: private::GetDataType,
{
    fn update_data(&self, message: DataMessage) -> Result<(), String> {
        match message {
            DataMessage::All(data_type, transport_type, data) => {
                check_data_type(self.data_type, data_type, &self.name)?;
                self.set_all(&data, transport_type)
            }
            DataMessage::BatchStart(count, data) => self.batch_start(&data, count),
            DataMessage::Batch(data) => self.batch(&data),
            DataMessage::BatchEnd(data_type, transport_type, data) => {
                check_data_type(self.data_type, data_type, &self.name)?;
                self.batch_end(&data, transport_type)
            }
            DataMessage::Drain(index, count) => self.drain(index, count),
            DataMessage::Clear => {
                self.inner.write().clear();
                self.buffer.lock().take();
                Ok(())
            }
        }
    }
}

impl<T> Clone for Data<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            inner: self.inner.clone(),
            buffer: self.buffer.clone(),
            sender: self.sender.clone(),
        }
    }
}

// MultiData -------------------------------------------------------------------
pub(crate) trait UpdateMultiData: Sync + Send {
    fn update(&self, key: u32, message: DataMessage) -> Result<(), String>;
    fn remove(&self, key: u32);
    fn reset(&self);
}

pub struct DataMulti<T> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    inner: Arc<RwLock<NoHashMap<u32, Vec<T>>>>,
    buffers: Arc<Mutex<NoHashMap<u32, Vec<T>>>>,
    sender: MessageSender,
}

#[allow(private_bounds)]
impl<T> DataMulti<T>
where
    T: private::GetDataType,
{
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            data_type: T::get_type(),
            element_size: T::get_type().element_size(),
            inner: Arc::new(RwLock::new(NoHashMap::default())),
            buffers: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
        }
    }

    #[inline]
    pub fn get(&self, key: u32) -> Option<Vec<T>> {
        self.inner.read().get(&key).cloned()
    }

    #[inline]
    pub fn read<R>(&self, key: u32, f: impl Fn(Option<&[T]>) -> R) -> R {
        self.inner
            .read()
            .get(&key)
            .map(|v| f(Some(&v)))
            .unwrap_or_else(|| f(None))
    }

    #[inline]
    pub fn read_all<R>(&self, f: impl Fn(&NoHashMap<u32, Vec<T>>) -> R) -> R {
        f(&self.inner.read())
    }

    #[inline]
    pub fn for_each<F>(&self, f: impl Fn(u32, &[T])) {
        self.inner.read().iter().for_each(|(k, v)| f(*k, &v));
    }

    fn set_all(&self, key: u32, data: &[u8], transport_type: TransportType) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));

        if data.len() % self.element_size != 0 {
            return Err(format!(
                "Data size {} is not a multiple of element size {}",
                data.len(),
                self.element_size
            ));
        }

        let count = data.len() / self.element_size;
        let mut buffer = Vec::with_capacity(count);
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                buffer.as_mut_ptr() as *mut u8,
                data.len(),
            );
            buffer.set_len(count);
        }
        self.save_data(key, buffer, transport_type)
    }

    fn batch_start(&self, key: u32, data: &[u8], elements_count: u64) -> Result<(), String> {
        let all_data_size = elements_count * self.element_size as u64;
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

        let mut buffer = Vec::with_capacity(elements_count as usize);
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

    fn batch(&self, key: u32, data: &[u8]) -> Result<(), String> {
        match self.buffers.lock().get_mut(&key) {
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
                "No header found for Data: {} when updating batch",
                self.name
            )),
        }
    }

    fn batch_end(
        &self,
        key: u32,
        data: &[u8],
        transport_type: TransportType,
    ) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));

        match self.buffers.lock().remove(&key) {
            Some(mut buffer) => {
                if data.len() % self.element_size != 0 {
                    return Err(format!(
                        "Batch data size {} is not a multiple of element size {}",
                        data.len(),
                        self.element_size
                    ));
                }
                let count = data.len() / self.element_size;

                if buffer.len() + count != buffer.capacity() {
                    return Err(format!(
                        "Batch end data size {} does not match total data size {}",
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

                self.save_data(key, buffer, transport_type)
            }
            None => Err(format!(
                "No header found for Data: {} when updating batch end",
                self.name
            )),
        }
    }

    fn drain(&self, key: u32, index: u64, count: u64) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));

        if let Some(data) = self.inner.write().get_mut(&key) {
            if index as usize + count as usize > data.len() {
                return Err(format!(
                    "Drain range ({} to {}) exceeds current data size {}",
                    index,
                    index + count,
                    data.len()
                ));
            }
            data.drain(index as usize..(index as usize + count as usize));
            return Ok(());
        }

        Err(format!(
            "Key {} does not exist for Drain transport type in MultiData: {}",
            key, self.name
        ))
    }

    fn save_data(
        &self,
        key: u32,
        data: Vec<T>,
        transport_type: TransportType,
    ) -> Result<(), String> {
        match transport_type {
            TransportType::Set(count) => {
                if data.len() as u64 != count {
                    return Err(format!(
                        "Data size {} does not match expected count {} for Set transport type",
                        data.len(),
                        count
                    ));
                }
                self.inner.write().insert(key, data);
            }
            TransportType::Add(count) => {
                if data.len() as u64 != count {
                    return Err(format!(
                        "Data size {} does not match expected count {} for Add transport type",
                        data.len(),
                        count
                    ));
                }
                match self.inner.write().entry(key) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().extend(data);
                    }
                    Entry::Vacant(_) => {
                        return Err(format!(
                            "Key {} does not exist for Add transport type in MultiData: {}",
                            key, self.name
                        ));
                    }
                }
            }
            TransportType::Replace(start, count) => {
                if data.len() as u64 != count {
                    return Err(format!(
                        "Data size {} does not match expected count {} for Replace transport type",
                        data.len(),
                        count
                    ));
                }
                match self.inner.write().entry(key) {
                    Entry::Occupied(mut entry) => {
                        let w = entry.get_mut();
                        if start as usize + data.len() > w.len() {
                            return Err(format!(
                                "Replace range ({} to {}) exceeds current data size {} for key {} in MultiData: {}",
                                start,
                                start + count,
                                w.len(),
                                key,
                                self.name
                            ));
                        }
                        w.splice(start as usize..(start as usize + data.len()), data);
                    }
                    Entry::Vacant(_) => {
                        return Err(format!(
                            "Key {} does not exist for Replace transport type in MultiData: {}",
                            key, self.name
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl<T> UpdateMultiData for DataMulti<T>
where
    T: private::GetDataType + Sync + Send,
{
    fn update(&self, key: u32, message: DataMessage) -> Result<(), String> {
        match message {
            DataMessage::All(data_type, transport_type, data) => {
                check_data_type(self.data_type, data_type, &self.name)?;
                self.set_all(key, &data, transport_type)
            }
            DataMessage::BatchStart(count, data) => self.batch_start(key, &data, count),
            DataMessage::Batch(data) => self.batch(key, &data),
            DataMessage::BatchEnd(data_type, transport_type, data) => {
                check_data_type(self.data_type, data_type, &self.name)?;
                self.batch_end(key, &data, transport_type)
            }
            DataMessage::Drain(index, count) => self.drain(key, index, count),
            DataMessage::Clear => {
                self.remove(key);
                Ok(())
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

impl<T> Clone for DataMulti<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            inner: self.inner.clone(),
            buffers: self.buffers.clone(),
            sender: self.sender.clone(),
        }
    }
}

// functions -------------------------------------------------------------------
#[inline]
fn check_data_type(expected: DataType, actual: DataType, name: &str) -> Result<(), String> {
    if expected != actual {
        return Err(format!(
            "Data type {:?} does not match expected type {:?} for Data: {}",
            actual, expected, name
        ));
    }
    Ok(())
}
