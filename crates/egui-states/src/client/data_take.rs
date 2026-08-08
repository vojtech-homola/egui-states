use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::client::data::private;
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

impl DataTakeMessage {
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(self, Self::All(..) | Self::BatchEnd(..))
    }
}

impl DataMultiTakeMessage {
    pub(crate) fn requires_ack_on_failure(&self) -> bool {
        match self {
            Self::Modify(_, message, blocking) => *blocking && message.is_terminal(),
            Self::Remove(_) | Self::Reset => false,
        }
    }
}

#[derive(Default)]
enum TakeBatch<T> {
    #[default]
    Empty,
    Receiving {
        buffer: Vec<T>,
        blocking: bool,
    },
    Failed {
        error: String,
        blocking: bool,
    },
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
    batch: Arc<Mutex<TakeBatch<T>>>,
    sender: MessageSender,
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
            batch: Arc::new(Mutex::new(TakeBatch::Empty)),
            sender,
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
}

impl<T> Clone for DataTake<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            inner: self.inner.clone(),
            batch: self.batch.clone(),
            sender: self.sender.clone(),
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
                let result = (|| {
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
                    Ok(buffer)
                })();

                match result {
                    Ok(buffer) => {
                        *self.inner.write() = Some((buffer, blocking));
                        Ok(())
                    }
                    Err(error) => {
                        if blocking {
                            self.sender.send_ack(self.id);
                        }
                        Err(error)
                    }
                }
            }
            DataTakeMessage::BatchStart(count, data) => {
                let result = (|| {
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
                    Ok(buffer)
                })();

                *self.batch.lock() = match result {
                    Ok(buffer) => TakeBatch::Receiving { buffer, blocking },
                    Err(error) => TakeBatch::Failed { error, blocking },
                };
                Ok(())
            }
            DataTakeMessage::Batch(data) => {
                let mut batch = self.batch.lock();
                let failure = match &mut *batch {
                    TakeBatch::Receiving {
                        buffer,
                        blocking: batch_blocking,
                    } => {
                        if *batch_blocking != blocking {
                            Some(format!(
                                "Blocking flag changed within a batch for DataTake: {}",
                                self.name
                            ))
                        } else if data.len() % self.element_size != 0 {
                            Some(format!(
                                "Batch data size {} is not a multiple of element size {}",
                                data.len(),
                                self.element_size
                            ))
                        } else {
                            let count = data.len() / self.element_size;
                            if buffer.len() + count > buffer.capacity() {
                                Some(format!(
                                    "Batch data size {} exceeds total data size {}",
                                    buffer.len() + count,
                                    buffer.capacity()
                                ))
                            } else {
                                unsafe {
                                    std::ptr::copy_nonoverlapping(
                                        data.as_ptr(),
                                        buffer.as_mut_ptr().add(buffer.len()) as *mut u8,
                                        data.len(),
                                    );
                                    buffer.set_len(buffer.len() + count);
                                }
                                None
                            }
                        }
                    }
                    TakeBatch::Failed {
                        blocking: batch_blocking,
                        ..
                    } => {
                        *batch_blocking |= blocking;
                        None
                    }
                    TakeBatch::Empty => Some(format!(
                        "No header found for DataTake: {} when updating batch",
                        self.name
                    )),
                };

                if let Some(error) = failure {
                    let ack_on_failure = match &*batch {
                        TakeBatch::Receiving {
                            blocking: batch_blocking,
                            ..
                        }
                        | TakeBatch::Failed {
                            blocking: batch_blocking,
                            ..
                        } => *batch_blocking || blocking,
                        TakeBatch::Empty => blocking,
                    };
                    *batch = TakeBatch::Failed {
                        error,
                        blocking: ack_on_failure,
                    };
                }
                Ok(())
            }
            DataTakeMessage::BatchEnd(data_type, count, data) => {
                let batch = std::mem::take(&mut *self.batch.lock());
                let (mut buffer, batch_blocking) = match batch {
                    TakeBatch::Receiving { buffer, blocking } => (buffer, blocking),
                    TakeBatch::Failed {
                        error,
                        blocking: batch_blocking,
                    } => {
                        if batch_blocking || blocking {
                            self.sender.send_ack(self.id);
                        }
                        return Err(error);
                    }
                    TakeBatch::Empty => {
                        if blocking {
                            self.sender.send_ack(self.id);
                        }
                        return Err(format!(
                            "No header found for DataTake: {} when updating batch end",
                            self.name
                        ));
                    }
                };

                let ack_on_failure = batch_blocking || blocking;
                let result = (|| {
                    if batch_blocking != blocking {
                        return Err(format!(
                            "Blocking flag changed within a batch for DataTake: {}",
                            self.name
                        ));
                    }
                    if data_type != self.data_type {
                        return Err(format!(
                            "Data type {:?} does not match expected type {:?} for DataTake: {}",
                            data_type, self.data_type, self.name
                        ));
                    }
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
                    Ok(())
                })();

                match result {
                    Ok(()) => {
                        *self.inner.write() = Some((buffer, blocking));
                        Ok(())
                    }
                    Err(error) => {
                        if ack_on_failure {
                            self.sender.send_ack(self.id);
                        }
                        Err(error)
                    }
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
    batches: Arc<Mutex<NoHashMap<u32, TakeBatch<T>>>>,
    sender: MessageSender,
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
            batches: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
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
}

impl<T> Clone for DataMultiTake<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            inner: self.inner.clone(),
            batches: self.batches.clone(),
            sender: self.sender.clone(),
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
                let result = (|| {
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
                    Ok(buffer)
                })();

                match result {
                    Ok(buffer) => {
                        self.inner.write().insert(key, (buffer, blocking));
                        Ok(())
                    }
                    Err(error) => {
                        if blocking {
                            self.sender.send_ack(self.id);
                        }
                        Err(error)
                    }
                }
            }
            DataTakeMessage::BatchStart(count, data) => {
                let result = (|| {
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
                    Ok(buffer)
                })();

                let batch = match result {
                    Ok(buffer) => TakeBatch::Receiving { buffer, blocking },
                    Err(error) => TakeBatch::Failed { error, blocking },
                };
                self.batches.lock().insert(key, batch);
                Ok(())
            }
            DataTakeMessage::Batch(data) => {
                let mut batches = self.batches.lock();
                let mut failure_blocking = blocking;
                let failure = match batches.get_mut(&key) {
                    Some(TakeBatch::Receiving {
                        buffer,
                        blocking: batch_blocking,
                    }) => {
                        failure_blocking |= *batch_blocking;
                        if *batch_blocking != blocking {
                            Some(format!(
                                "Blocking flag changed within a batch for DataMultiTake: {} key {}",
                                self.name, key
                            ))
                        } else if data.len() % self.element_size != 0 {
                            Some(format!(
                                "Batch data size {} is not a multiple of element size {}",
                                data.len(),
                                self.element_size
                            ))
                        } else {
                            let count = data.len() / self.element_size;
                            if buffer.len() + count > buffer.capacity() {
                                Some(format!(
                                    "Batch data size {} exceeds total data size {}",
                                    buffer.len() + count,
                                    buffer.capacity()
                                ))
                            } else {
                                unsafe {
                                    std::ptr::copy_nonoverlapping(
                                        data.as_ptr(),
                                        buffer.as_mut_ptr().add(buffer.len()) as *mut u8,
                                        data.len(),
                                    );
                                    buffer.set_len(buffer.len() + count);
                                }
                                None
                            }
                        }
                    }
                    Some(TakeBatch::Failed {
                        blocking: batch_blocking,
                        ..
                    }) => {
                        *batch_blocking |= blocking;
                        None
                    }
                    Some(TakeBatch::Empty) | None => Some(format!(
                        "No header found for DataMultiTake: {} key {} when updating batch",
                        self.name, key
                    )),
                };

                if let Some(error) = failure {
                    batches.insert(
                        key,
                        TakeBatch::Failed {
                            error,
                            blocking: failure_blocking,
                        },
                    );
                }
                Ok(())
            }
            DataTakeMessage::BatchEnd(data_type, count, data) => {
                let batch = self.batches.lock().remove(&key).unwrap_or_default();
                let (mut buffer, batch_blocking) = match batch {
                    TakeBatch::Receiving { buffer, blocking } => (buffer, blocking),
                    TakeBatch::Failed {
                        error,
                        blocking: batch_blocking,
                    } => {
                        if batch_blocking || blocking {
                            self.sender.send_ack(self.id);
                        }
                        return Err(error);
                    }
                    TakeBatch::Empty => {
                        if blocking {
                            self.sender.send_ack(self.id);
                        }
                        return Err(format!(
                            "No header found for DataMultiTake: {} key {} when updating batch end",
                            self.name, key
                        ));
                    }
                };

                let ack_on_failure = batch_blocking || blocking;
                let result = (|| {
                    if batch_blocking != blocking {
                        return Err(format!(
                            "Blocking flag changed within a batch for DataMultiTake: {} key {}",
                            self.name, key
                        ));
                    }
                    if data_type != self.data_type {
                        return Err(format!(
                            "Data type {:?} does not match expected type {:?} for DataMultiTake: {}",
                            data_type, self.data_type, self.name
                        ));
                    }
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
                    Ok(())
                })();

                match result {
                    Ok(()) => {
                        self.inner.write().insert(key, (buffer, blocking));
                        Ok(())
                    }
                    Err(error) => {
                        if ack_on_failure {
                            self.sender.send_ack(self.id);
                        }
                        Err(error)
                    }
                }
            }
        }
    }

    fn remove(&self, key: u32) {
        self.inner.write().remove(&key);
        self.batches.lock().remove(&key);
    }

    fn reset(&self) {
        self.inner.write().clear();
        self.batches.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::messages::{ChannelMessage, MessageSender};
    use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

    fn assert_single_ack(
        receiver: &mut UnboundedReceiver<Option<ChannelMessage>>,
        expected_id: u64,
    ) {
        match receiver.try_recv() {
            Ok(Some(ChannelMessage::Ack(id))) => assert_eq!(id, expected_id),
            _ => panic!("expected an ACK for {expected_id}"),
        }
        assert_no_message(receiver);
    }

    fn assert_no_message(receiver: &mut UnboundedReceiver<Option<ChannelMessage>>) {
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn data_take_acks_only_blocking_single_message_failures() {
        let id = 61;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u8>::new("take".to_string(), id, sender);

        let invalid = || DataTakeMessage::All(DataType::U16, 1, Bytes::from_static(&[0, 0]));
        assert!(take.update(invalid(), true).is_err());
        assert_single_ack(&mut receiver, id);

        assert!(take.update(invalid(), false).is_err());
        assert_no_message(&mut receiver);
    }

    #[test]
    fn failed_data_take_batch_acks_at_end_and_keeps_previous_value() {
        let id = 62;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u16>::new("take".to_string(), id, sender);

        take.update(
            DataTakeMessage::All(
                DataType::U16,
                1,
                Bytes::copy_from_slice(&9_u16.to_ne_bytes()),
            ),
            false,
        )
        .unwrap();
        take.update(
            DataTakeMessage::BatchStart(3, Bytes::copy_from_slice(&1_u16.to_ne_bytes())),
            true,
        )
        .unwrap();
        take.update(DataTakeMessage::Batch(Bytes::from_static(&[1])), true)
            .unwrap();
        assert_no_message(&mut receiver);

        assert!(
            take.update(
                DataTakeMessage::BatchEnd(
                    DataType::U16,
                    3,
                    Bytes::copy_from_slice(&2_u16.to_ne_bytes()),
                ),
                true,
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);
        assert_eq!(take.take(), Some(vec![9]));
        assert_no_message(&mut receiver);
    }

    #[test]
    fn successful_blocking_data_take_still_acks_on_consumption() {
        let id = 63;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u8>::new("take".to_string(), id, sender);

        take.update(
            DataTakeMessage::All(DataType::U8, 2, Bytes::from_static(&[1, 2])),
            true,
        )
        .unwrap();
        assert_no_message(&mut receiver);
        assert_eq!(take.take(), Some(vec![1, 2]));
        assert_single_ack(&mut receiver, id);
    }

    #[test]
    fn data_multi_take_acks_blocking_single_and_batch_failures() {
        let id = 64;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataMultiTake::<u16>::new("multi_take".to_string(), id, sender);

        assert!(
            take.update(
                3,
                DataTakeMessage::All(DataType::U8, 1, Bytes::from_static(&[0])),
                true,
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);

        take.update(
            7,
            DataTakeMessage::BatchStart(3, Bytes::copy_from_slice(&1_u16.to_ne_bytes())),
            true,
        )
        .unwrap();
        take.update(7, DataTakeMessage::Batch(Bytes::from_static(&[1])), true)
            .unwrap();
        assert_no_message(&mut receiver);
        assert!(
            take.update(
                7,
                DataTakeMessage::BatchEnd(
                    DataType::U16,
                    3,
                    Bytes::copy_from_slice(&2_u16.to_ne_bytes()),
                ),
                true,
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);
        assert!(!take.is_some(7));
    }
}
