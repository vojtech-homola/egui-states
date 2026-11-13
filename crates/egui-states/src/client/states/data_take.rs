//! One-shot client handles for numeric buffers sent by the server.

use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::client::messages::{ChannelMessage, MessageSender};
use crate::client::states::data::{
    BatchBuffer, HeaderValue, created_header, decode_header_bytes, elements, header_type_id,
    private, split_header,
};
use crate::hashing::NoHashMap;
use crate::protocol::data::{DataTakeHeader, DataType, HeaderBytes};

/// A completed transfer, its header and whether taking it must acknowledge
/// it.
type Completed<T, H> = (Vec<T>, H, bool);

/// Splits a terminal message's payload into its header and its elements and
/// decodes the header; only a state without a header type takes a transfer
/// without one.
fn take_header<'a, H: HeaderValue>(
    type_id: Option<u32>,
    header: Option<HeaderBytes>,
    payload: &'a [u8],
    name: &str,
) -> Result<(H, &'a [u8]), String> {
    match header {
        None => Ok((created_header(type_id, None, name)?, payload)),
        Some(header) => {
            let (bytes, data) = split_header(payload, header.size, name)?;
            Ok((decode_header_bytes(type_id, header, bytes, name)?, data))
        }
    }
}

/// A take message: its wire header and the payload that followed it.
pub(crate) type DataTakeMessage = (DataTakeHeader, Bytes);

#[derive(Default)]
enum TakeBatch<T> {
    #[default]
    Empty,
    Receiving {
        buffer: BatchBuffer<T>,
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
    /// The server releases blocking sends when a connection ends, so pending
    /// values from it must not be acknowledged on the next connection.
    fn disconnected(&self);
}

/// A one-shot numeric buffer sent from the server to the client, with an
/// optional typed header `H`.
///
/// A transfer becomes available only after every batch has arrived. Taking a
/// blocking transfer also sends its acknowledgement to the server. Every
/// transfer of a state with a header type comes with its header.
pub struct DataTake<T, H = ()> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    header_type: Option<u32>,
    inner: Arc<RwLock<Option<Completed<T, H>>>>,
    batch: Arc<Mutex<TakeBatch<T>>>,
    sender: MessageSender,
}

#[allow(private_bounds)]
impl<T, H> DataTake<T, H>
where
    T: private::GetDataType,
    H: HeaderValue,
{
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            data_type: T::get_type(),
            element_size: T::get_type().item_size(),
            header_type: header_type_id::<H>(),
            inner: Arc::new(RwLock::new(None)),
            batch: Arc::new(Mutex::new(TakeBatch::Empty)),
            sender,
        }
    }

    /// Removes and returns the completed pending buffer, if one has arrived.
    ///
    /// Each buffer can be taken once. Taking a blocking transfer acknowledges
    /// it and allows the server to send the next pending transfer.
    pub fn take(&self) -> Option<Vec<T>> {
        self.take_with_header().map(|(value, _)| value)
    }

    /// Like [`Self::take`], with the header the buffer was sent with.
    pub fn take_with_header(&self) -> Option<(Vec<T>, H)> {
        let (value, header, blocking) = self.inner.write().take()?;
        if blocking {
            self.sender.send(ChannelMessage::Ack(self.id));
        }
        Some((value, header))
    }

    /// Returns whether a completed buffer is waiting to be taken.
    ///
    /// A batch still being received does not count as pending.
    pub fn is_some(&self) -> bool {
        self.inner.read().is_some()
    }
}

impl<T, H> Clone for DataTake<T, H> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            header_type: self.header_type,
            inner: self.inner.clone(),
            batch: self.batch.clone(),
            sender: self.sender.clone(),
        }
    }
}

#[allow(private_bounds)]
impl<T, H> UpdateDataTake for DataTake<T, H>
where
    T: private::GetDataType + Send + Sync,
    H: HeaderValue,
{
    fn update(&self, (header, data): DataTakeMessage, blocking: bool) -> Result<(), String> {
        match header {
            DataTakeHeader::All(data_type, count, _, _, header) => {
                let result = (|| {
                    if data_type != self.data_type {
                        return Err(format!(
                            "Data type {:?} does not match expected type {:?} for DataTake: {}",
                            data_type, self.data_type, self.name
                        ));
                    }
                    let (header, data) =
                        take_header::<H>(self.header_type, header, &data, &self.name)?;
                    if count.checked_mul(self.element_size as u64) != Some(data.len() as u64) {
                        return Err(format!(
                            "Data size {} does not match expected count {} for DataTake: {}",
                            data.len(),
                            count,
                            self.name
                        ));
                    }
                    Ok((elements(data)?, header))
                })();

                match result {
                    Ok((buffer, header)) => {
                        *self.inner.write() = Some((buffer, header, blocking));
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
            DataTakeHeader::StartBatch(count, _) => {
                let result = BatchBuffer::start(&data, count);

                *self.batch.lock() = match result {
                    Ok(buffer) => TakeBatch::Receiving { buffer, blocking },
                    Err(error) => TakeBatch::Failed { error, blocking },
                };
                Ok(())
            }
            DataTakeHeader::Batch(_) => {
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
                        } else {
                            buffer.append(&data).err()
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
            DataTakeHeader::End(data_type, count, _, _, header) => {
                let batch = std::mem::take(&mut *self.batch.lock());
                let (buffer, batch_blocking) = match batch {
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
                    // The batch start's count bounds the copy; the End count
                    // must agree with it as well.
                    let (header, data) =
                        take_header::<H>(self.header_type, header, &data, &self.name)?;
                    let buffer = buffer.finish(data)?;
                    if buffer.len() as u64 != count {
                        return Err(format!(
                            "Batch end data size {} does not match total data size {}",
                            buffer.len(),
                            count
                        ));
                    }
                    Ok((buffer, header))
                })();

                match result {
                    Ok((buffer, header)) => {
                        *self.inner.write() = Some((buffer, header, blocking));
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

    fn disconnected(&self) {
        if let Some((_, _, blocking)) = self.inner.write().as_mut() {
            *blocking = false;
        }
        // A batch from the closed connection can never complete.
        *self.batch.lock() = TakeBatch::Empty;
    }
}

// DataMultiTake ----------------------------------------------------------------
pub(crate) trait UpdateDataMultiTake: Sync + Send {
    fn update(&self, key: u32, message: DataTakeMessage, blocking: bool) -> Result<(), String>;
    fn remove(&self, key: u32);
    fn reset(&self);
    /// The server releases blocking sends when a connection ends, so pending
    /// values from it must not be acknowledged on the next connection.
    fn disconnected(&self);
}

/// Keyed one-shot numeric buffers sent from the server to the client.
///
/// Each `u32` key has an independent pending transfer; taking one key does not
/// consume or acknowledge any other key.
pub struct DataMultiTake<T, H = ()> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    header_type: Option<u32>,
    inner: Arc<RwLock<NoHashMap<u32, Completed<T, H>>>>,
    batches: Arc<Mutex<NoHashMap<u32, TakeBatch<T>>>>,
    sender: MessageSender,
}

#[allow(private_bounds)]
impl<T, H> DataMultiTake<T, H>
where
    T: private::GetDataType,
    H: HeaderValue,
{
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            data_type: T::get_type(),
            element_size: T::get_type().item_size(),
            header_type: header_type_id::<H>(),
            inner: Arc::new(RwLock::new(NoHashMap::default())),
            batches: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
        }
    }

    /// Removes and returns the completed pending buffer at `key`, if present.
    ///
    /// Taking a blocking transfer acknowledges it and allows the server to send
    /// the next transfer for that key.
    pub fn take(&self, key: u32) -> Option<Vec<T>> {
        self.take_with_header(key).map(|(value, _)| value)
    }

    /// Like [`Self::take`], with the header the buffer was sent with.
    pub fn take_with_header(&self, key: u32) -> Option<(Vec<T>, H)> {
        let (value, header, blocking) = self.inner.write().remove(&key)?;
        if blocking {
            self.sender.send(ChannelMessage::Ack(self.id));
        }
        Some((value, header))
    }

    /// Returns whether a completed buffer at `key` is waiting to be taken.
    pub fn is_some(&self, key: u32) -> bool {
        self.inner.read().contains_key(&key)
    }
}

impl<T, H> Clone for DataMultiTake<T, H> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            header_type: self.header_type,
            inner: self.inner.clone(),
            batches: self.batches.clone(),
            sender: self.sender.clone(),
        }
    }
}

#[allow(private_bounds)]
impl<T, H> UpdateDataMultiTake for DataMultiTake<T, H>
where
    T: private::GetDataType + Send + Sync,
    H: HeaderValue,
{
    fn update(
        &self,
        key: u32,
        (header, data): DataTakeMessage,
        blocking: bool,
    ) -> Result<(), String> {
        match header {
            DataTakeHeader::All(data_type, count, _, _, header) => {
                let result = (|| {
                    if data_type != self.data_type {
                        return Err(format!(
                            "Data type {:?} does not match expected type {:?} for DataMultiTake: {}",
                            data_type, self.data_type, self.name
                        ));
                    }
                    let (header, data) =
                        take_header::<H>(self.header_type, header, &data, &self.name)?;
                    if count.checked_mul(self.element_size as u64) != Some(data.len() as u64) {
                        return Err(format!(
                            "Data size {} does not match expected count {} for DataMultiTake: {}",
                            data.len(),
                            count,
                            self.name
                        ));
                    }
                    Ok((elements(data)?, header))
                })();

                match result {
                    Ok((buffer, header)) => {
                        self.inner.write().insert(key, (buffer, header, blocking));
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
            DataTakeHeader::StartBatch(count, _) => {
                let result = BatchBuffer::start(&data, count);

                let batch = match result {
                    Ok(buffer) => TakeBatch::Receiving { buffer, blocking },
                    Err(error) => TakeBatch::Failed { error, blocking },
                };
                self.batches.lock().insert(key, batch);
                Ok(())
            }
            DataTakeHeader::Batch(_) => {
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
                        } else {
                            buffer.append(&data).err()
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
            DataTakeHeader::End(data_type, count, _, _, header) => {
                let batch = self.batches.lock().remove(&key).unwrap_or_default();
                let (buffer, batch_blocking) = match batch {
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
                    // The batch start's count bounds the copy; the End count
                    // must agree with it as well.
                    let (header, data) =
                        take_header::<H>(self.header_type, header, &data, &self.name)?;
                    let buffer = buffer.finish(data)?;
                    if buffer.len() as u64 != count {
                        return Err(format!(
                            "Batch end data size {} does not match total data size {}",
                            buffer.len(),
                            count
                        ));
                    }
                    Ok((buffer, header))
                })();

                match result {
                    Ok((buffer, header)) => {
                        self.inner.write().insert(key, (buffer, header, blocking));
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

    // A discarded blocking buffer will never be taken, so acknowledge it here or
    // the server keeps waiting before it sends the next transfer.
    fn remove(&self, key: u32) {
        let removed = self.inner.write().remove(&key);
        self.batches.lock().remove(&key);
        if let Some((_, _, true)) = removed {
            self.sender.send_ack(self.id);
        }
    }

    fn reset(&self) {
        let blocking = self
            .inner
            .write()
            .drain()
            .filter(|(_, (_, _, blocking))| *blocking)
            .count();
        self.batches.lock().clear();
        for _ in 0..blocking {
            self.sender.send_ack(self.id);
        }
    }

    fn disconnected(&self) {
        for (_, _, blocking) in self.inner.write().values_mut() {
            *blocking = false;
        }
        // Batches from the closed connection can never complete.
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

        let invalid = || {
            (
                DataTakeHeader::All(DataType::U16, 1, false, 0, None),
                Bytes::from_static(&[0, 0]),
            )
        };
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
            (
                DataTakeHeader::All(DataType::U16, 1, false, 0, None),
                Bytes::copy_from_slice(&9_u16.to_ne_bytes()),
            ),
            false,
        )
        .unwrap();
        take.update(
            (
                DataTakeHeader::StartBatch(3, 0),
                Bytes::copy_from_slice(&1_u16.to_ne_bytes()),
            ),
            true,
        )
        .unwrap();
        take.update((DataTakeHeader::Batch(0), Bytes::from_static(&[1])), true)
            .unwrap();
        assert_no_message(&mut receiver);

        assert!(
            take.update(
                (
                    DataTakeHeader::End(DataType::U16, 3, false, 0, None),
                    Bytes::copy_from_slice(&2_u16.to_ne_bytes())
                ),
                true,
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);
        assert_eq!(take.take(), Some(vec![9]));
        assert_no_message(&mut receiver);
    }

    /// Counts come from the network, so absurd values must be errors rather
    /// than panics, aborts, or short buffers.
    #[test]
    fn oversized_wire_counts_are_rejected() {
        let (sender, _receiver) = MessageSender::new();
        let take = DataTake::<u16>::new("take".to_string(), 65, sender.clone());
        let multi = DataMultiTake::<u16>::new("takes".to_string(), 65, sender);

        for count in [u64::MAX, u64::MAX / 2 + 1, 1 << 62] {
            let all = || {
                (
                    DataTakeHeader::All(DataType::U16, count, false, 0, None),
                    Bytes::new(),
                )
            };
            assert!(take.update(all(), false).is_err());
            assert!(multi.update(4, all(), false).is_err());

            let start = || {
                (
                    DataTakeHeader::StartBatch(count, 0),
                    Bytes::from_static(&[1, 0]),
                )
            };
            let end = || {
                (
                    DataTakeHeader::End(DataType::U16, count, false, 0, None),
                    Bytes::new(),
                )
            };
            take.update(start(), false).unwrap();
            assert!(take.update(end(), false).is_err());
            multi.update(4, start(), false).unwrap();
            assert!(multi.update(4, end(), false).is_err());
        }
        assert_eq!(take.take(), None);
        assert_eq!(multi.take(4), None);
    }

    /// An End count larger than the Start count must not let the final chunk
    /// be copied past the buffer the start allocated (D2).
    #[test]
    fn a_batch_end_larger_than_its_start_is_rejected() {
        let id = 66;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u16>::new("take".to_string(), id, sender.clone());
        let multi = DataMultiTake::<u16>::new("takes".to_string(), id, sender);
        let tail = Bytes::from(vec![7; 200]);
        for blocking in [false, true] {
            let start = || {
                (
                    DataTakeHeader::StartBatch(2, 0),
                    Bytes::from_static(&[1, 0]),
                )
            };
            let end = || {
                (
                    DataTakeHeader::End(DataType::U16, 101, false, 0, None),
                    tail.clone(),
                )
            };

            let acked_once = |receiver: &mut UnboundedReceiver<Option<ChannelMessage>>| {
                if blocking {
                    assert_single_ack(receiver, id);
                } else {
                    assert_no_message(receiver);
                }
            };

            take.update(start(), blocking).unwrap();
            let error = take.update(end(), blocking).unwrap_err();
            assert!(error.contains("exceeds total data size 2"), "{error}");
            acked_once(&mut receiver);
            multi.update(4, start(), blocking).unwrap();
            let error = multi.update(4, end(), blocking).unwrap_err();
            assert!(error.contains("exceeds total data size 2"), "{error}");
            acked_once(&mut receiver);
        }
        assert_eq!(take.take(), None);
        assert_eq!(multi.take(4), None);
    }

    #[test]
    fn successful_blocking_data_take_still_acks_on_consumption() {
        let id = 63;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u8>::new("take".to_string(), id, sender);

        take.update(
            (
                DataTakeHeader::All(DataType::U8, 2, false, 0, None),
                Bytes::from_static(&[1, 2]),
            ),
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
                (
                    DataTakeHeader::All(DataType::U8, 1, false, 0, None),
                    Bytes::from_static(&[0])
                ),
                true,
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);

        take.update(
            7,
            (
                DataTakeHeader::StartBatch(3, 0),
                Bytes::copy_from_slice(&1_u16.to_ne_bytes()),
            ),
            true,
        )
        .unwrap();
        take.update(
            7,
            (DataTakeHeader::Batch(0), Bytes::from_static(&[1])),
            true,
        )
        .unwrap();
        assert_no_message(&mut receiver);
        assert!(
            take.update(
                7,
                (
                    DataTakeHeader::End(DataType::U16, 3, false, 0, None),
                    Bytes::copy_from_slice(&2_u16.to_ne_bytes())
                ),
                true,
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);
        assert!(!take.is_some(7));
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    fn discard_pending(reset: bool) {
        let (sender, mut receiver) = MessageSender::new();
        let data = DataMultiTake::<u8>::new("takes".into(), 93, sender);
        data.update(
            7,
            (
                DataTakeHeader::All(DataType::U8, 1, false, 0, None),
                Bytes::from_static(&[7]),
            ),
            true,
        )
        .unwrap();
        assert!(receiver.try_recv().is_err());
        if reset {
            data.reset();
        } else {
            data.remove(7);
        }
        assert!(data.take(7).is_none());
        crate::test_support::ack(&mut receiver, 93);
        data.reset();
        data.remove(7);
        assert!(
            receiver.try_recv().is_err(),
            "discarding twice must not ACK twice"
        );
        data.update(
            513,
            (
                DataTakeHeader::All(DataType::U8, 1, false, 0, None),
                Bytes::from_static(&[9]),
            ),
            true,
        )
        .unwrap();
        assert_eq!(data.take(513), Some(vec![9]));
        crate::test_support::ack(&mut receiver, 93);
        assert!(data.take(513).is_none());
    }

    #[test]
    fn removing_pending_blocking_key_acknowledges_once() {
        discard_pending(false);
    }
    #[test]
    fn resetting_pending_blocking_key_acknowledges_once() {
        discard_pending(true);
    }

    /// The server releases blocking sends on disconnect; acknowledging a buffer
    /// from the closed connection would release a later send on the next one.
    #[test]
    fn buffers_from_a_closed_connection_are_not_acknowledged() {
        let (sender, mut receiver) = MessageSender::new();
        let single = DataTake::<u8>::new("take".into(), 94, sender.clone());
        let multi = DataMultiTake::<u8>::new("takes".into(), 95, sender);
        let message = || {
            (
                DataTakeHeader::All(DataType::U8, 1, false, 0, None),
                Bytes::from_static(&[7]),
            )
        };
        single.update(message(), true).unwrap();
        multi.update(7, message(), true).unwrap();
        multi.update(8, message(), true).unwrap();

        single.disconnected();
        multi.disconnected();
        multi.reset();
        assert_eq!(single.take(), Some(vec![7]));
        assert!(
            receiver.try_recv().is_err(),
            "a stale buffer was acknowledged"
        );
    }
}
