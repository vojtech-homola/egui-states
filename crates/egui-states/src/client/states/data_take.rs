//! One-shot client handles for numeric buffers sent by the server.

use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::client::messages::MessageSender;
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

/// A batched transfer of one key that is still arriving.
#[derive(Default)]
enum TakeBatch<T> {
    #[default]
    Empty,
    Receiving {
        buffer: BatchBuffer<T>,
        blocking: bool,
    },
    Failed(String),
}

/// What a take state checks a transfer against.
struct TakeSpec<'a> {
    /// The state and key, for errors: `DataTake: name` or
    /// `DataMultiTake: name key 3`.
    label: &'a str,
    data_type: DataType,
    element_size: usize,
    header_type: Option<u32>,
}

/// Receives one message of a transfer into `batch`. Returns the completed
/// buffer and header for the message that ends the transfer, the error for a
/// transfer that failed, and `None` while the transfer is still arriving.
fn receive<T, H>(
    spec: &TakeSpec,
    batch: &mut TakeBatch<T>,
    (header, data): DataTakeMessage,
    blocking: bool,
) -> Option<Result<(Vec<T>, H), String>>
where
    T: private::GetDataType,
    H: HeaderValue,
{
    let check_type = |data_type: DataType| {
        if data_type == spec.data_type {
            Ok(())
        } else {
            Err(format!(
                "Data type {:?} does not match expected type {:?} for {}",
                data_type, spec.data_type, spec.label
            ))
        }
    };
    match header {
        DataTakeHeader::All(data_type, count, _, header) => Some((|| {
            check_type(data_type)?;
            let (header, data) = take_header::<H>(spec.header_type, header, &data, spec.label)?;
            if count.checked_mul(spec.element_size as u64) != Some(data.len() as u64) {
                return Err(format!(
                    "Data size {} does not match expected count {} for {}",
                    data.len(),
                    count,
                    spec.label
                ));
            }
            Ok((elements(data)?, header))
        })()),
        DataTakeHeader::StartBatch(count, _) => {
            *batch = match BatchBuffer::start(&data, count) {
                Ok(buffer) => TakeBatch::Receiving { buffer, blocking },
                Err(error) => TakeBatch::Failed(error),
            };
            None
        }
        DataTakeHeader::Batch(_) => {
            let failure = match batch {
                TakeBatch::Receiving {
                    buffer,
                    blocking: batch_blocking,
                } => {
                    if *batch_blocking != blocking {
                        Some(format!(
                            "Blocking flag changed within a batch for {}",
                            spec.label
                        ))
                    } else {
                        buffer.append(&data).err()
                    }
                }
                TakeBatch::Failed(_) => None,
                TakeBatch::Empty => Some(format!(
                    "No header found for {} when updating batch",
                    spec.label
                )),
            };
            if let Some(error) = failure {
                *batch = TakeBatch::Failed(error);
            }
            None
        }
        DataTakeHeader::End(data_type, count, _, header) => {
            let (buffer, batch_blocking) = match std::mem::take(batch) {
                TakeBatch::Receiving { buffer, blocking } => (buffer, blocking),
                TakeBatch::Failed(error) => return Some(Err(error)),
                TakeBatch::Empty => {
                    return Some(Err(format!(
                        "No header found for {} when updating batch end",
                        spec.label
                    )));
                }
            };
            Some((|| {
                if batch_blocking != blocking {
                    return Err(format!(
                        "Blocking flag changed within a batch for {}",
                        spec.label
                    ));
                }
                check_type(data_type)?;
                // The batch start's count bounds the copy; the End count must
                // agree with it as well.
                let (header, data) = take_header::<H>(spec.header_type, header, &data, spec.label)?;
                let buffer = buffer.finish(data)?;
                if buffer.len() as u64 != count {
                    return Err(format!(
                        "Batch end data size {} does not match total data size {}",
                        buffer.len(),
                        count
                    ));
                }
                Ok((buffer, header))
            })())
        }
    }
}

// DataTake --------------------------------------------------------------------
pub(crate) trait UpdateDataTake: Sync + Send {
    fn update(&self, message: DataTakeMessage, blocking: bool) -> Result<(), String>;
    /// The server releases blocking sends when a connection ends, so pending
    /// values from it must not be acknowledged on the next connection.
    fn disconnected(&self);
    /// Drops the buffer a closed connection left untaken, without
    /// acknowledging it.
    fn connected(&self);
}

/// A one-shot numeric buffer sent from the server to the client, with an
/// optional typed header `H`.
///
/// A transfer becomes available only after every batch has arrived, and a
/// new one replaces a buffer that was not taken. The client acknowledges a
/// transfer when it arrives, so the server sends the next one; a blocking
/// transfer only once it is taken. An untaken buffer stays takeable while the
/// client is disconnected, and is dropped once a new connection is accepted.
/// Every transfer of a state with a header type comes with its header.
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
            self.sender.send_ack(self.id);
        }
        Some((value, header))
    }

    /// Returns whether a completed buffer is waiting to be taken.
    ///
    /// A batch still being received does not count as pending.
    pub fn is_some(&self) -> bool {
        self.inner.read().is_some()
    }

    /// Drops the buffer waiting to be taken, if any. A blocking one is
    /// acknowledged, as taking it would.
    pub fn clear(&self) {
        if let Some((_, _, true)) = self.inner.write().take() {
            self.sender.send_ack(self.id);
        }
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
    fn update(&self, message: DataTakeMessage, blocking: bool) -> Result<(), String> {
        let label = format!("DataTake: {}", self.name);
        let spec = TakeSpec {
            label: &label,
            data_type: self.data_type,
            element_size: self.element_size,
            header_type: self.header_type,
        };
        let completed = receive::<T, H>(&spec, &mut self.batch.lock(), message, blocking);
        // Every transfer is acknowledged once: a failed one and a
        // non-blocking one now, a blocking one when it is taken.
        match completed {
            None => Ok(()),
            Some(Ok((buffer, header))) => {
                *self.inner.write() = Some((buffer, header, blocking));
                if !blocking {
                    self.sender.send_ack(self.id);
                }
                Ok(())
            }
            Some(Err(error)) => {
                self.sender.send_ack(self.id);
                Err(error)
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

    fn connected(&self) {
        *self.inner.write() = None;
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
    /// Drops the buffers a closed connection left untaken, without
    /// acknowledging them.
    fn connected(&self);
}

/// The buffers of a `DataMultiTake` waiting to be taken, each with the
/// sequence number of its key's first untaken arrival.
struct MultiPending<T, H> {
    values: NoHashMap<u32, (Completed<T, H>, u64)>,
    next: u64,
}

impl<T, H> MultiPending<T, H> {
    fn insert(&mut self, key: u32, completed: Completed<T, H>) {
        // A key that already waits keeps its place.
        let sequence = match self.values.get(&key) {
            Some((_, sequence)) => *sequence,
            None => {
                self.next += 1;
                self.next
            }
        };
        self.values.insert(key, (completed, sequence));
    }

    fn remove(&mut self, key: u32) -> Option<Completed<T, H>> {
        self.values.remove(&key).map(|(completed, _)| completed)
    }

    /// The key that has waited longest.
    fn oldest(&self) -> Option<u32> {
        self.values
            .iter()
            .min_by_key(|(_, (_, sequence))| *sequence)
            .map(|(key, _)| *key)
    }
}

/// Keyed one-shot numeric buffers sent from the server to the client.
///
/// Each `u32` key has an independent pending transfer, acknowledged as for
/// [`DataTake`]; taking one key does not consume or acknowledge any other key.
pub struct DataMultiTake<T, H = ()> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    header_type: Option<u32>,
    inner: Arc<RwLock<MultiPending<T, H>>>,
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
            inner: Arc::new(RwLock::new(MultiPending {
                values: NoHashMap::default(),
                next: 0,
            })),
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
        let (value, header, blocking) = self.inner.write().remove(key)?;
        if blocking {
            self.sender.send_ack_key(self.id, key);
        }
        Some((value, header))
    }

    /// Removes and returns the buffer that has waited longest, with its key,
    /// so the UI can take every waiting buffer without knowing the keys:
    ///
    /// ```no_run
    /// # fn show(key: u32, frame: Vec<u8>) {}
    /// # fn ui(frames: &egui_states::DataMultiTake<u8>) {
    /// while let Some((key, frame)) = frames.take_next() {
    ///     show(key, frame);
    /// }
    /// # }
    /// ```
    ///
    /// A key's place is that of the first buffer that arrived since it was
    /// last taken; a newer buffer of the key replaces that one and keeps
    /// the place. Taking a blocking transfer acknowledges it, as
    /// [`Self::take`] does.
    pub fn take_next(&self) -> Option<(u32, Vec<T>)> {
        self.take_next_with_header()
            .map(|(key, value, _)| (key, value))
    }

    /// Like [`Self::take_next`], with the header the buffer was sent with.
    pub fn take_next_with_header(&self) -> Option<(u32, Vec<T>, H)> {
        let (key, (value, header, blocking)) = {
            let mut pending = self.inner.write();
            let key = pending.oldest()?;
            (key, pending.remove(key)?)
        };
        if blocking {
            self.sender.send_ack_key(self.id, key);
        }
        Some((key, value, header))
    }

    /// Returns whether a completed buffer at `key` is waiting to be taken.
    pub fn is_some(&self, key: u32) -> bool {
        self.inner.read().values.contains_key(&key)
    }

    /// Returns the number of keys with a buffer waiting to be taken.
    pub fn len(&self) -> usize {
        self.inner.read().values.len()
    }

    /// Returns whether no buffer waits to be taken.
    pub fn is_empty(&self) -> bool {
        self.inner.read().values.is_empty()
    }

    /// Drops every buffer waiting to be taken. Blocking ones are
    /// acknowledged, as taking them would.
    pub fn clear(&self) {
        for key in self.drain_blocking() {
            self.sender.send_ack_key(self.id, key);
        }
    }

    /// Drops every waiting buffer and returns the keys of the blocking ones.
    fn drain_blocking(&self) -> Vec<u32> {
        self.inner
            .write()
            .values
            .drain()
            .filter(|(_, ((_, _, blocking), _))| *blocking)
            .map(|(key, _)| key)
            .collect()
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
    fn update(&self, key: u32, message: DataTakeMessage, blocking: bool) -> Result<(), String> {
        let label = format!("DataMultiTake: {} key {}", self.name, key);
        let spec = TakeSpec {
            label: &label,
            data_type: self.data_type,
            element_size: self.element_size,
            header_type: self.header_type,
        };
        let completed = {
            let mut batches = self.batches.lock();
            let batch = batches.entry(key).or_default();
            let completed = receive::<T, H>(&spec, batch, message, blocking);
            if matches!(batch, TakeBatch::Empty) {
                batches.remove(&key);
            }
            completed
        };
        // Acknowledged as in `DataTake::update`, for the key.
        match completed {
            None => Ok(()),
            Some(Ok((buffer, header))) => {
                self.inner.write().insert(key, (buffer, header, blocking));
                if !blocking {
                    self.sender.send_ack_key(self.id, key);
                }
                Ok(())
            }
            Some(Err(error)) => {
                self.sender.send_ack_key(self.id, key);
                Err(error)
            }
        }
    }

    // A discarded blocking buffer will never be taken, so acknowledge it here or
    // the server keeps waiting before it sends the next transfer.
    fn remove(&self, key: u32) {
        let removed = self.inner.write().remove(key);
        self.batches.lock().remove(&key);
        if let Some((_, _, true)) = removed {
            self.sender.send_ack_key(self.id, key);
        }
    }

    fn reset(&self) {
        self.batches.lock().clear();
        self.clear();
    }

    fn disconnected(&self) {
        for ((_, _, blocking), _) in self.inner.write().values.values_mut() {
            *blocking = false;
        }
        // Batches from the closed connection can never complete.
        self.batches.lock().clear();
    }

    fn connected(&self) {
        self.inner.write().values.clear();
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
            Ok(Some(ChannelMessage::Ack(id) | ChannelMessage::AckKey(id, _))) => {
                assert_eq!(id, expected_id)
            }
            _ => panic!("expected an ACK for {expected_id}"),
        }
        assert_no_message(receiver);
    }

    fn assert_no_message(receiver: &mut UnboundedReceiver<Option<ChannelMessage>>) {
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn a_failed_data_take_is_acknowledged_once() {
        let id = 61;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u8>::new("take".to_string(), id, sender);

        let invalid = || {
            (
                DataTakeHeader::All(DataType::U16, 1, 0, None),
                Bytes::from_static(&[0, 0]),
            )
        };
        for blocking in [true, false] {
            assert!(take.update(invalid(), blocking).is_err());
            assert_single_ack(&mut receiver, id);
        }
    }

    #[test]
    fn failed_data_take_batch_acks_at_end_and_keeps_previous_value() {
        let id = 62;
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u16>::new("take".to_string(), id, sender);

        take.update(
            (
                DataTakeHeader::All(DataType::U16, 1, 0, None),
                Bytes::copy_from_slice(&9_u16.to_ne_bytes()),
            ),
            false,
        )
        .unwrap();
        // A non-blocking transfer is acknowledged when it arrives.
        assert_single_ack(&mut receiver, id);
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
                    DataTakeHeader::End(DataType::U16, 3, 0, None),
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
                    DataTakeHeader::All(DataType::U16, count, 0, None),
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
                    DataTakeHeader::End(DataType::U16, count, 0, None),
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
                    DataTakeHeader::End(DataType::U16, 101, 0, None),
                    tail.clone(),
                )
            };

            let acked_once = |receiver: &mut UnboundedReceiver<Option<ChannelMessage>>| {
                assert_single_ack(receiver, id);
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
                DataTakeHeader::All(DataType::U8, 2, 0, None),
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
    fn a_non_blocking_data_take_is_acknowledged_when_it_arrives() {
        let (sender, mut receiver) = MessageSender::new();
        let take = DataTake::<u8>::new("take".to_string(), 67, sender.clone());
        let multi = DataMultiTake::<u8>::new("takes".to_string(), 68, sender);
        let message = || {
            (
                DataTakeHeader::All(DataType::U8, 1, 0, None),
                Bytes::from_static(&[1]),
            )
        };
        take.update(message(), false).unwrap();
        assert_single_ack(&mut receiver, 67);
        multi.update(4, message(), false).unwrap();
        assert!(matches!(
            receiver.try_recv(),
            Ok(Some(ChannelMessage::AckKey(68, 4)))
        ));
        assert_eq!(take.take(), Some(vec![1]));
        assert_eq!(multi.take(4), Some(vec![1]));
        assert_no_message(&mut receiver);
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
                    DataTakeHeader::All(DataType::U8, 1, 0, None),
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
                    DataTakeHeader::End(DataType::U16, 3, 0, None),
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
                DataTakeHeader::All(DataType::U8, 1, 0, None),
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
                DataTakeHeader::All(DataType::U8, 1, 0, None),
                Bytes::from_static(&[9]),
            ),
            true,
        )
        .unwrap();
        assert_eq!(data.take(513), Some(vec![9]));
        crate::test_support::ack(&mut receiver, 93);
        assert!(data.take(513).is_none());
    }

    fn arrive(data: &DataMultiTake<u8>, key: u32, value: u8, blocking: bool) {
        data.update(
            key,
            (
                DataTakeHeader::All(DataType::U8, 1, 0, None),
                Bytes::copy_from_slice(&[value]),
            ),
            blocking,
        )
        .unwrap();
    }

    /// `take_next` returns the key that has waited longest; a newer buffer
    /// of a waiting key replaces it and keeps its place.
    #[test]
    fn take_next_returns_the_buffers_in_arrival_order() {
        let (sender, mut receiver) = MessageSender::new();
        let data = DataMultiTake::<u8>::new("takes".into(), 96, sender);
        assert_eq!(data.take_next(), None);
        arrive(&data, 9, 1, false);
        arrive(&data, 2, 2, false);
        arrive(&data, 5, 3, true);
        arrive(&data, 9, 4, false);
        assert_eq!(data.len(), 3);
        while let Ok(Some(_)) = receiver.try_recv() {}

        assert_eq!(data.take_next(), Some((9, vec![4])));
        assert_eq!(data.take_next(), Some((2, vec![2])));
        assert!(receiver.try_recv().is_err());
        // A key taken and arriving again goes to the back.
        arrive(&data, 9, 5, false);
        assert!(matches!(
            receiver.try_recv(),
            Ok(Some(crate::client::messages::ChannelMessage::AckKey(96, 9)))
        ));
        assert_eq!(data.take_next(), Some((5, vec![3])));
        crate::test_support::ack(&mut receiver, 96);
        assert_eq!(data.take_next(), Some((9, vec![5])));
        assert_eq!(data.take_next(), None);
        assert!(data.is_empty());
    }

    /// `clear` drops every waiting buffer and acknowledges the blocking ones
    /// once, as taking them would.
    #[test]
    fn clear_acknowledges_the_blocking_buffers_once() {
        let (sender, mut receiver) = MessageSender::new();
        let multi = DataMultiTake::<u8>::new("takes".into(), 97, sender.clone());
        let single = DataTake::<u8>::new("take".into(), 98, sender);
        arrive(&multi, 1, 1, false);
        arrive(&multi, 2, 2, true);
        crate::test_support::ack(&mut receiver, 97);
        multi.clear();
        crate::test_support::ack(&mut receiver, 97);
        multi.clear();
        assert!(receiver.try_recv().is_err());
        assert!(multi.is_empty());

        single
            .update(
                (
                    DataTakeHeader::All(DataType::U8, 1, 0, None),
                    Bytes::from_static(&[3]),
                ),
                true,
            )
            .unwrap();
        single.clear();
        crate::test_support::ack(&mut receiver, 98);
        assert!(!single.is_some());
        single.clear();
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn removing_pending_blocking_key_acknowledges_once() {
        discard_pending(false);
    }
    #[test]
    fn resetting_pending_blocking_key_acknowledges_once() {
        discard_pending(true);
    }

    /// Buffers stay takeable while the client is disconnected, and are
    /// dropped once a new connection is accepted, without an
    /// acknowledgement.
    #[test]
    fn a_new_connection_drops_untaken_buffers() {
        let (sender, mut receiver) = MessageSender::new();
        let single = DataTake::<u8>::new("take".into(), 99, sender.clone());
        let multi = DataMultiTake::<u8>::new("takes".into(), 100, sender);
        let message = || {
            (
                DataTakeHeader::All(DataType::U8, 1, 0, None),
                Bytes::from_static(&[7]),
            )
        };
        single.update(message(), true).unwrap();
        multi.update(7, message(), true).unwrap();
        single.disconnected();
        multi.disconnected();
        assert!(single.is_some() && multi.is_some(7));

        single.connected();
        multi.connected();
        assert_eq!(single.take(), None);
        assert_eq!(multi.take_next(), None);
        assert!(receiver.try_recv().is_err());
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
                DataTakeHeader::All(DataType::U8, 1, 0, None),
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
