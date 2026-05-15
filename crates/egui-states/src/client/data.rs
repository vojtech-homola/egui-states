use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::client::event::EventUniversal;
use crate::client::messages::{ChannelMessage, MessageSender};
use crate::data_transport::{DataType, TransportType};

pub(crate) enum DataMessage {
    All(DataType, TransportType, Bytes),
    BatchStart(u64, Bytes),
    Batch(Bytes),
    BatchEnd(DataType, TransportType, Bytes),
    Drain(u64, u64),
    Clear,
}

pub(crate) enum DataTakeMessage {
    All(DataType, u64, Bytes),
    BatchStart(u64, Bytes),
    Batch(Bytes),
    BatchEnd(DataType, u64, Bytes),
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
            element_size: T::get_type().item_size(),
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

// DataTake --------------------------------------------------------------------
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
    pub fn wait(&self) {
        while self.inner.read().is_none() {
            self.event.wait_clear_blocking();
        }
    }

    pub async fn wait_async(&self) {
        while self.inner.read().is_none() {
            self.event.wait_clear().await;
        }
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

pub(crate) trait UpdateDataTake: Sync + Send {
    fn update_take(&self, message: DataTakeMessage, blocking: bool) -> Result<(), String>;
}

#[allow(private_bounds)]
impl<T> UpdateDataTake for DataTake<T>
where
    T: private::GetDataType + Send + Sync,
{
    fn update_take(&self, message: DataTakeMessage, blocking: bool) -> Result<(), String> {
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

// functions -------------------------------------------------------------------
#[inline]
pub(crate) fn check_data_type(expected: DataType, actual: DataType, name: &str) -> Result<(), String> {
    if expected != actual {
        return Err(format!(
            "Data type {:?} does not match expected type {:?} for Data: {}",
            actual, expected, name
        ));
    }
    Ok(())
}
