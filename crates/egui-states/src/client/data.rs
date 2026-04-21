use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::client::messages::MessageSender;
use crate::data_transport::{DataType, TransportType};

pub(crate) enum DataMessage {
    All(DataType, TransportType, Bytes),
    BatchStart(u64, Bytes),
    Batch(Bytes),
    BatchEnd(DataType, TransportType, Bytes),
    Drain(u64, u64),
    Clear,
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

pub(crate) trait UpdateData: Sync + Send {
    fn update_data(&self, message: DataMessage) -> Result<(), String>;
}

pub struct Data<T> {
    name: String,
    id: u64,
    data_type: DataType,
    element_size: usize,
    inner: Arc<RwLock<(Vec<T>, bool)>>,
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
            name,
            id,
            data_type: T::get_type(),
            element_size: T::get_type().element_size(),
            inner: Arc::new(RwLock::new((Vec::new(), false))),
            buffer: Arc::new(Mutex::new(None)),
            sender,
        }
    }

    pub fn get(&self) -> Vec<T> {
        let inner = self.inner.read();
        inner.0.clone()
    }

    pub fn get_updated(&self) -> Option<Vec<T>> {
        let mut inner = self.inner.write();
        if inner.1 {
            inner.1 = false;
            Some(inner.0.clone())
        } else {
            None
        }
    }

    pub fn read<R>(&self, f: impl Fn((&[T], bool)) -> R) -> R {
        let mut inner = self.inner.write();
        let result = f((&inner.0, inner.1));
        inner.1 = false;
        result
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
                *self.inner.write() = (data, true);
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
                w.0.extend(data);
                w.1 = true;
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
                if start as usize + data.len() > w.0.len() {
                    return Err(format!(
                        "Replace range ({} to {}) exceeds current data size {}",
                        start,
                        start + count,
                        w.0.len()
                    ));
                }
                w.0.splice(start as usize..(start as usize + data.len()), data);
                w.1 = true;
            }
        }
        Ok(())
    }

    fn set_all(&self, data: &[u8], transport_type: TransportType) -> Result<(), String> {
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
        let mut w = self.inner.write();
        if index as usize + count as usize > w.0.len() {
            return Err(format!(
                "Drain range ({} to {}) exceeds current data size {}",
                index,
                index + count,
                w.0.len()
            ));
        }
        w.0.drain(index as usize..(index as usize + count as usize));
        w.1 = true;

        Ok(())
    }

    fn clear(&self) {
        let mut w = self.inner.write();
        w.0.clear();
        w.1 = true;
    }
}

impl<T: Sync + Send> UpdateData for Data<T>
where
    T: private::GetDataType,
{
    fn update_data(&self, message: DataMessage) -> Result<(), String> {
        match message {
            DataMessage::All(data_type, transport_type, data) => {
                if data_type != self.data_type {
                    return Err(format!(
                        "Data type {:?} does not match expected type {:?} for Data: {}",
                        data_type, self.data_type, self.name
                    ));
                }
                self.set_all(&data, transport_type)
            }
            DataMessage::BatchStart(count, data) => self.batch_start(&data, count),
            DataMessage::Batch(data) => self.batch(&data),
            DataMessage::BatchEnd(data_type, transport_type, data) => {
                if data_type != self.data_type {
                    return Err(format!(
                        "Data type {:?} does not match expected type {:?} for Data: {}",
                        data_type, self.data_type, self.name
                    ));
                }
                self.batch_end(&data, transport_type)
            }
            DataMessage::Drain(index, count) => self.drain(index, count),
            DataMessage::Clear => {
                self.clear();
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
