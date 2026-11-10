use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::data_transport::{DataMultiTakeHeader, DataTakeHeader, DataType};
use crate::event::Event;
use crate::hashing::NoHashMap;
use crate::serialization::{FastVec, TRANSFER_CHUNK_SIZE};
use crate::server_core::data_core::{DataHolder, SyncState, check_data_type};
use crate::server_core::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::transfer::{Chunk, chunk_transfer};

// DataTake --------------------------------------------------
struct TakeState {
    cache: Option<(Vec<u8>, usize)>,
    sync: SyncState,
}

/// Server side of a one-shot numeric transfer.
///
/// A producer waits for the client to take a pending blocking value before it
/// takes the state lock, so a remove or reset can still cancel that value.
/// It then stores the cache and queues its chunks under the lock. While the
/// state awaits a synchronization with a newly connected client, or when its
/// wait spanned a reset, it only stores: the synchronization sends the cache,
/// and sending as well would deliver the value twice.
pub(crate) struct DataTake {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    event: Event,
    state: Mutex<TakeState>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl DataTake {
    pub(crate) fn new(
        name: String,
        id: u64,
        data_type: DataType,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            data_type,
            item_size: data_type.item_size(),
            event: Event::new(),
            state: Mutex::new(TakeState {
                cache: None,
                sync: SyncState::new(),
            }),
            sender,
            connected,
        })
    }

    pub(crate) fn set(
        &self,
        data: DataHolder,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let generation = self.state.lock().sync.generation();

        let messages = if self.connected.load(Ordering::Acquire) {
            let messages = pack_data_take(
                self.id,
                slice,
                data.count as u64,
                self.data_type,
                blocking,
                update,
            )?;
            wait_for_slot(&self.event, blocking);
            Some(messages)
        } else {
            None
        };

        let mut state = self.state.lock();
        state.cache = cache.then(|| (slice.to_vec(), data.count));
        if let Some(messages) = messages {
            send_if_current(
                &state.sync,
                &self.event,
                &self.connected,
                &self.sender,
                generation,
                blocking,
                messages,
            );
        }
        Ok(())
    }
}

impl Acknowledge for DataTake {
    fn acknowledge(&self) {
        self.event.set();
    }

    fn reset(&self) {
        self.state.lock().sync.reset();
        self.event.set();
    }
}

impl SyncTrait for DataTake {
    fn sync(&self) -> Result<(), ()> {
        let mut state = self.state.lock();

        if let Some((ref data, count)) = state.cache {
            let messages =
                pack_data_take(self.id, data, count as u64, self.data_type, false, false)
                    .map_err(|_| ())?;

            for (message, single) in messages {
                self.sender.send_set(message, single);
            }
        }

        state.sync.synchronized();
        self.event.set();

        Ok(())
    }
}

// DataMultiTake --------------------------------------------------
struct MultiTakeState {
    values: NoHashMap<u32, (Vec<u8>, usize)>,
    sync: SyncState,
}

/// One-shot numeric transfers indexed by key; see [`DataTake`] for the
/// producer protocol. All keys share one acknowledgement stream.
pub(crate) struct DataMultiTake {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    state: RwLock<MultiTakeState>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl DataMultiTake {
    pub(crate) fn new(
        name: String,
        id: u64,
        data_type: DataType,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            data_type,
            item_size: data_type.item_size(),
            state: RwLock::new(MultiTakeState {
                values: NoHashMap::default(),
                sync: SyncState::new(),
            }),
            sender,
            connected,
            event: Event::new(),
        })
    }

    pub(crate) fn set(
        &self,
        index: u32,
        data: DataHolder,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let generation = self.state.read().sync.generation();

        let messages = if self.connected.load(Ordering::Acquire) {
            let messages = pack_data_multi_take(
                self.id,
                index,
                slice,
                data.count as u64,
                self.data_type,
                blocking,
                update,
            )?;
            wait_for_slot(&self.event, blocking);
            Some(messages)
        } else {
            None
        };

        // Held until every chunk is queued, so concurrent non-blocking
        // transfers cannot interleave their chunks on the wire.
        let mut w = self.state.write();
        if cache {
            w.values.insert(index, (slice.to_vec(), data.count));
        } else {
            w.values.remove(&index);
        }
        if let Some(messages) = messages {
            send_if_current(
                &w.sync,
                &self.event,
                &self.connected,
                &self.sender,
                generation,
                blocking,
                messages,
            );
        }
        Ok(())
    }

    pub(crate) fn remove_index(&self, index: u32, update: bool) -> Result<(), String> {
        let mut w = self.state.write();
        w.values.remove(&index);
        if w.sync.sendable(&self.connected).is_some() {
            let header = DataMultiTakeHeader::Remove(index, update);
            let message = header
                .serialize(self.id)
                .map_err(|_| "Failed to serialize remove index header".to_string())?;
            self.sender.send(message);
        }

        Ok(())
    }

    pub(crate) fn reset(&self, update: bool) -> Result<(), String> {
        let mut w = self.state.write();
        w.values.clear();
        if w.sync.sendable(&self.connected).is_some() {
            let header = DataMultiTakeHeader::Reset(update);
            let message = header
                .serialize(self.id)
                .map_err(|_| "Failed to serialize reset header".to_string())?;
            self.sender.send(message);
        }

        Ok(())
    }
}

impl Acknowledge for DataMultiTake {
    fn acknowledge(&self) {
        self.event.set();
    }

    fn reset(&self) {
        self.state.write().sync.reset();
        self.event.set();
    }
}

impl SyncTrait for DataMultiTake {
    fn sync(&self) -> Result<(), ()> {
        let mut w = self.state.write();
        w.sync.synchronized();
        if w.values.is_empty() {
            let message = DataMultiTakeHeader::Reset(false).serialize(self.id)?;
            self.sender.send(message);
            self.event.set();
        } else {
            for (index, (data, count)) in w.values.iter() {
                let messages = pack_data_multi_take(
                    self.id,
                    *index,
                    data,
                    *count as u64,
                    self.data_type,
                    false,
                    false,
                )
                .map_err(|_| ())?;

                for (message, single) in messages {
                    self.sender.send_set(message, single);
                }

                if !self.connected.load(Ordering::Acquire) {
                    return Ok(());
                }
            }
            self.event.set();
        }

        Ok(())
    }
}

/// Waits until no blocking transfer is pending. A blocking send takes the
/// slot, so the next send waits until the client took this one; a
/// non-blocking send leaves it free.
fn wait_for_slot(event: &Event, blocking: bool) {
    match blocking {
        true => event.wait_clear(),
        false => event.wait(),
    }
}

/// Queues `messages` if the state is still synchronized in the generation the
/// producer started in, under the caller's state lock. Otherwise the
/// synchronization delivers the cached value, and a blocking producer returns
/// the slot its wait consumed.
fn send_if_current(
    sync: &SyncState,
    event: &Event,
    connected: &AtomicBool,
    sender: &MessageSender,
    generation: u64,
    blocking: bool,
    messages: Vec<(FastVec<32>, bool)>,
) {
    if sync.sendable(connected) != Some(generation) {
        if blocking {
            event.set();
        }
        return;
    }
    for (message, single) in messages {
        sender.send_set(message, single);
    }
}

fn pack_data_take(
    id: u64,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_data_take_chunked(
        id,
        data,
        data_count,
        data_type,
        blocking,
        update,
        TRANSFER_CHUNK_SIZE,
    )
}

/// Packs a one-shot transfer; `wrap` serializes its header for the state.
fn pack_take(
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    update: bool,
    chunk_limit: usize,
    wrap: impl Fn(DataTakeHeader) -> Result<FastVec<32>, String>,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    chunk_transfer(
        data.len(),
        data_type.item_size(),
        chunk_limit,
        |chunk, range| {
            let len = range.len() as u32;
            let header = match chunk {
                Chunk::All => DataTakeHeader::All(data_type, data_count, update, len),
                Chunk::Start => DataTakeHeader::StartBatch(data_count, len),
                Chunk::Batch => DataTakeHeader::Batch(len),
                Chunk::End => DataTakeHeader::End(data_type, data_count, update, len),
            };
            let mut message = wrap(header)?;
            message.reserve_exact(range.len());
            message.extend_from_slice(&data[range]);
            Ok(message)
        },
    )
}

pub(super) fn pack_data_take_chunked(
    id: u64,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_take(data, data_count, data_type, update, chunk_limit, |header| {
        header
            .serialize(id, blocking, true)
            .map_err(|_| "Failed to serialize DataTake header".to_string())
    })
}

fn pack_data_multi_take(
    id: u64,
    index: u32,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_data_multi_take_chunked(
        id,
        index,
        data,
        data_count,
        data_type,
        blocking,
        update,
        TRANSFER_CHUNK_SIZE,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn pack_data_multi_take_chunked(
    id: u64,
    index: u32,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_take(data, data_count, data_type, update, chunk_limit, |header| {
        DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
            .map_err(|_| "Failed to serialize DataMultiTake header".to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn holder(data: &[u8]) -> DataHolder {
        DataHolder {
            data: data.as_ptr(),
            count: data.len(),
            data_size: data.len(),
            data_type: DataType::U8,
        }
    }

    fn count(receiver: &mut crate::server_core::sender::MessageReceiver) -> usize {
        let mut count = 0;
        while let Ok(Some(_)) = receiver.try_recv() {
            count += 1;
        }
        count
    }

    fn within<T: Send + 'static>(label: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(f()).unwrap());
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| panic!("blocked: {label}"))
    }

    /// A cached take set in the reconnect window used to be queued by the
    /// producer and again by the synchronization from the cache (D8).
    #[test]
    fn a_cached_take_in_the_reconnect_window_is_delivered_once() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let take = DataTake::new(
            "root.take".into(),
            5,
            DataType::U8,
            sender,
            connected.clone(),
        );
        connected.store(true, Ordering::Release);
        take.sync().unwrap();
        take.set(holder(&[1]), false, false, true).unwrap();
        assert_eq!(count(&mut receiver), 1);

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*take);
        connected.store(true, Ordering::Release);
        // Blocking: it consumes the slot the reset set, and must return it.
        take.set(holder(&[2]), true, false, true).unwrap();
        assert_eq!(count(&mut receiver), 0, "a window take was sent");
        take.sync().unwrap();
        assert_eq!(
            count(&mut receiver),
            1,
            "the synchronization sends the cache"
        );

        // The slot is free again: one blocking take goes out, the next waits.
        let first = take.clone();
        within("first blocking take after sync", move || {
            first.set(holder(&[3]), true, false, true)
        })
        .unwrap();
        assert_eq!(count(&mut receiver), 1);
        let second = take.clone();
        let waiting = std::thread::spawn(move || second.set(holder(&[4]), true, false, true));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(count(&mut receiver), 0, "a blocking take did not wait");
        take.acknowledge();
        within("second blocking take", move || waiting.join().unwrap()).unwrap();
        assert_eq!(count(&mut receiver), 1);
    }

    #[test]
    fn a_cached_multi_take_in_the_reconnect_window_is_delivered_once() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let take = DataMultiTake::new(
            "root.take".into(),
            5,
            DataType::U8,
            sender,
            connected.clone(),
        );
        connected.store(true, Ordering::Release);
        take.sync().unwrap();
        assert_eq!(count(&mut receiver), 1, "empty synchronization resets");
        take.set(1, holder(&[1]), false, false, true).unwrap();
        assert_eq!(count(&mut receiver), 1);

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*take);
        connected.store(true, Ordering::Release);
        take.set(1, holder(&[2]), true, false, true).unwrap();
        take.remove_index(3, false).unwrap();
        assert_eq!(count(&mut receiver), 0, "a window change was sent");
        take.sync().unwrap();
        assert_eq!(
            count(&mut receiver),
            1,
            "the synchronization sends the cache"
        );

        let first = take.clone();
        within("first blocking multi take after sync", move || {
            first.set(1, holder(&[3]), true, false, true)
        })
        .unwrap();
        assert_eq!(count(&mut receiver), 1);
    }
}

#[cfg(all(test, feature = "client"))]
mod transfer_tests {
    use super::*;
    use crate::client::data_take::{
        DataMultiTake as ClientMulti, DataTake as ClientTake, UpdateDataMultiTake, UpdateDataTake,
    };
    use crate::client::messages::{MessageSender as ClientSender, MessagesParser};
    use crate::serialization::ServerHeader;

    #[test]
    fn real_take_chunks_publish_once_and_ack_only_after_consumption() {
        for size in [
            TRANSFER_CHUNK_SIZE - 1,
            TRANSFER_CHUNK_SIZE,
            TRANSFER_CHUNK_SIZE + 1,
            2 * TRANSFER_CHUNK_SIZE + 17,
        ] {
            let expected: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            for key in [None, Some(113)] {
                for blocking in [false, true] {
                    let (sender, mut receiver) = ClientSender::new();
                    let single = ClientTake::<u8>::new("take".into(), 61, sender.clone());
                    let multi = ClientMulti::<u8>::new("takes".into(), 61, sender);
                    for _ in 0..2 {
                        let messages = match key {
                            None => pack_data_take(
                                61,
                                &expected,
                                size as u64,
                                DataType::U8,
                                blocking,
                                true,
                            ),
                            Some(key) => pack_data_multi_take(
                                61,
                                key,
                                &expected,
                                size as u64,
                                DataType::U8,
                                blocking,
                                true,
                            ),
                        }
                        .unwrap();
                        let total = messages.len();
                        assert_eq!(total, size.div_ceil(TRANSFER_CHUNK_SIZE));
                        for (n, (message, _)) in messages.into_iter().enumerate() {
                            let (mut parser, (header, payload)) =
                                MessagesParser::from_bytes(message.into_bytes()).unwrap();
                            assert_eq!(header.update_flag(), n + 1 == total);
                            match header {
                                ServerHeader::DataTake(61, header, actual_blocking) => {
                                    assert_eq!(actual_blocking, blocking);
                                    single.update((header, payload), actual_blocking).unwrap();
                                }
                                ServerHeader::DataMultiTake(
                                    61,
                                    DataMultiTakeHeader::Modify(113, header, actual_blocking),
                                ) => {
                                    assert_eq!(actual_blocking, blocking);
                                    multi
                                        .update(113, (header, payload), actual_blocking)
                                        .unwrap();
                                }
                                _ => panic!("wrong packed take message"),
                            }
                            assert!(parser.next().unwrap().is_none());
                            assert!(receiver.try_recv().is_err(), "ACK before consumption");
                            if n + 1 < total {
                                assert!(single.take().is_none());
                                assert!(multi.take(113).is_none());
                            }
                        }
                        let actual = if key.is_none() {
                            single.take()
                        } else {
                            multi.take(113)
                        };
                        assert_eq!(actual.as_deref(), Some(expected.as_slice()));
                        if blocking {
                            crate::test_support::ack(&mut receiver, 61);
                        }
                        assert!(single.take().is_none());
                        assert!(multi.take(113).is_none());
                        assert!(multi.take(0).is_none());
                        assert!(receiver.try_recv().is_err());
                    }
                }
            }
        }
    }
}
