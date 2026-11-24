use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use parking_lot::Mutex;

use crate::hashing::NoHashMap;
use crate::protocol::data::{DataMultiTakeHeader, DataTakeHeader, DataType, HeaderBytes};
use crate::protocol::{FastVec, TRANSFER_CHUNK_SIZE};
use crate::server_core::error::CoreError;
use crate::server_core::net::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::states::data::{DataSlice, check_data_type};
use crate::server_core::states::header::HeaderType;
use crate::server_core::states::scheduler::Scheduler;
use crate::server_core::transfer::{Chunk, chunk_transfer};

// DataTake --------------------------------------------------
/// A cached transfer: its elements, their count and its encoded header.
type Cached = (Vec<u8>, usize, Option<Vec<u8>>);

/// The single key of a `DataTake` in its scheduler.
const KEY: u32 = 0;

/// Server side of a one-shot numeric transfer.
///
/// Transfers are sent one at a time, as for `Data`: a producer waits until
/// the client acknowledged the previous transfer, which it does when the
/// transfer arrives, or when the UI takes it for a blocking one. A producer
/// stores the cache and checks whether it may send under the state lock, so
/// a disconnect either invalidates its transfer or leaves it only stored:
/// while no client is synchronized nothing is sent and nothing waits, and
/// the synchronization sends the cache.
pub(crate) struct DataTake {
    pub(crate) name: String,
    pub(crate) data_type: DataType,
    item_size: usize,
    header_type: HeaderType,
    cache: Mutex<Option<Cached>>,
    scheduler: Scheduler,
}

impl DataTake {
    pub(crate) fn new(
        name: String,
        id: u64,
        data_type: DataType,
        header_type: Option<u32>,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            data_type,
            item_size: data_type.item_size(),
            header_type: HeaderType::new("Data", header_type),
            cache: Mutex::new(None),
            scheduler: Scheduler::new(id, sender, connected),
        })
    }

    pub(crate) fn set(
        &self,
        data: DataSlice,
        blocking: bool,
        cache: bool,
        wait: bool,
    ) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        self.header_type
            .require(&self.name, header, "with every take")?;
        let slice = data.bytes;

        let lock = self.scheduler.lock(KEY);
        let generation = {
            let mut stored = self.cache.lock();
            *stored = cache.then(|| (slice.to_vec(), elements, header.map(<[u8]>::to_vec)));
            self.scheduler.sendable()
        };
        let Some(generation) = generation else {
            return Ok(());
        };
        let messages = pack_data_take(
            self.scheduler.id,
            slice,
            elements as u64,
            self.data_type,
            blocking,
            take_header(&self.header_type, header),
        )?;
        self.scheduler.send_set(lock, messages, wait, generation);
        Ok(())
    }

    /// Whether no transfer waits for the client's acknowledgement, so the
    /// next one is sent at once. Also true while no client is synchronized.
    pub(crate) fn is_idle(&self) -> bool {
        self.scheduler.is_idle()
    }
}

impl Acknowledge for DataTake {
    fn acknowledge(&self) {
        self.scheduler.acknowledge(KEY);
    }

    fn reset(&self) {
        // Under the state lock, so a producer either checked whether it may
        // send before this reset, or sees the new state.
        let _cache = self.cache.lock();
        self.scheduler.reset();
    }
}

impl SyncTrait for DataTake {
    fn sync(&self) -> Result<(), ()> {
        // See `Data::sync` for why every key is locked.
        let lock = self.scheduler.lock_all();
        let cache = self.cache.lock();
        match &*cache {
            Some((data, count, header)) => {
                let messages = pack_data_take(
                    self.scheduler.id,
                    data,
                    *count as u64,
                    self.data_type,
                    false,
                    take_header(&self.header_type, header.as_deref()),
                )
                .map_err(|_| ())?;
                self.scheduler.begin_sync(&lock, messages, &[KEY]);
            }
            None => self.scheduler.begin_sync(&lock, Vec::new(), &[]),
        }
        Ok(())
    }
}

// DataMultiTake --------------------------------------------------
/// One-shot numeric transfers indexed by key; see [`DataTake`] for the
/// producer protocol, which runs for each key on its own: the keys do not
/// wait for each other's acknowledgements.
pub(crate) struct DataMultiTake {
    pub(crate) name: String,
    pub(crate) data_type: DataType,
    item_size: usize,
    header_type: HeaderType,
    cache: Mutex<NoHashMap<u32, Cached>>,
    scheduler: Scheduler,
}

impl DataMultiTake {
    pub(crate) fn new(
        name: String,
        id: u64,
        data_type: DataType,
        header_type: Option<u32>,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            data_type,
            item_size: data_type.item_size(),
            header_type: HeaderType::new("Data", header_type),
            cache: Mutex::new(NoHashMap::default()),
            scheduler: Scheduler::new(id, sender, connected),
        })
    }

    pub(crate) fn set(
        &self,
        index: u32,
        data: DataSlice,
        blocking: bool,
        cache: bool,
        wait: bool,
    ) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        self.header_type
            .require(&self.name, header, "with every take")?;
        let slice = data.bytes;

        // The cache changes when the value is queued, not before the wait
        // for the key: a removal or a reset in between must not leave the
        // cache and the client with different values. The storage lock is
        // returned to the scheduler, which holds it until the set is queued.
        let store = || {
            let mut stored = self.cache.lock();
            if cache {
                let cached = (slice.to_vec(), elements, header.map(<[u8]>::to_vec));
                stored.insert(index, cached);
            } else {
                stored.remove(&index);
            }
            stored
        };

        let lock = self.scheduler.lock(index);
        // A synchronization waits for the key's lock, so it sends what is
        // stored here.
        let Some(generation) = self.scheduler.sendable() else {
            drop(store());
            return Ok(());
        };
        let messages = pack_data_multi_take(
            self.scheduler.id,
            index,
            slice,
            elements as u64,
            self.data_type,
            blocking,
            take_header(&self.header_type, header),
        )?;
        self.scheduler
            .send_set_stored(lock, messages, wait, generation, store);
        Ok(())
    }

    /// Removes `index`: its cache, and the value the client has not taken.
    ///
    /// It is queued at once, without waiting for the transfers of `index`: a
    /// blocking one is acknowledged only when the UI takes it, and the client
    /// acknowledges it when it removes it.
    pub(crate) fn remove_index(&self, index: u32) -> Result<(), String> {
        let message = DataMultiTakeHeader::Remove(index)
            .serialize(self.scheduler.id)
            .map_err(|_| "Failed to serialize remove index header".to_string())?;
        // Queued under the storage lock, so the cache and the wire see this
        // removal and the sets of `index` in the same order.
        let mut stored = self.cache.lock();
        stored.remove(&index);
        if let Some(generation) = self.scheduler.sendable() {
            self.scheduler.send_now(message, generation);
        }
        Ok(())
    }

    /// Removes every key, as [`remove_index`](Self::remove_index) one.
    pub(crate) fn reset(&self) -> Result<(), String> {
        let message = DataMultiTakeHeader::Reset
            .serialize(self.scheduler.id)
            .map_err(|_| "Failed to serialize reset header".to_string())?;
        // See `remove_index` for the storage lock.
        let mut stored = self.cache.lock();
        stored.clear();
        if let Some(generation) = self.scheduler.sendable() {
            self.scheduler.send_now(message, generation);
        }
        Ok(())
    }

    /// Whether no transfer of any key waits for the client's
    /// acknowledgement.
    pub(crate) fn is_idle(&self) -> bool {
        self.scheduler.is_idle()
    }

    /// Whether no transfer of `index` waits for the client's
    /// acknowledgement.
    pub(crate) fn is_index_idle(&self, index: u32) -> bool {
        self.scheduler.is_key_idle(index)
    }
}

impl Acknowledge for DataMultiTake {
    /// The client acknowledges each key on its own, with
    /// [`acknowledge_key`](Acknowledge::acknowledge_key).
    fn acknowledge(&self) {}

    fn acknowledge_key(&self, key: u32) {
        self.scheduler.acknowledge(key);
    }

    fn reset(&self) {
        // Under the state lock, so a producer either checked whether it may
        // send before this reset, or sees the new state.
        let _cache = self.cache.lock();
        self.scheduler.reset();
    }
}

impl SyncTrait for DataMultiTake {
    fn sync(&self) -> Result<(), ()> {
        // See `Data::sync` for why every key is locked.
        let lock = self.scheduler.lock_all();
        let cache = self.cache.lock();

        // The client dropped its untaken values when the connection was
        // accepted; the reset says so on the wire as well, as for a
        // DataMulti. Each cached key is packed and queued on its own.
        let message = DataMultiTakeHeader::Reset.serialize(self.scheduler.id)?;
        let keys = cache.keys().copied().collect::<Vec<_>>();
        self.scheduler
            .begin_sync(&lock, vec![(message, false)], &keys);
        for (index, (data, count, header)) in cache.iter() {
            let messages = pack_data_multi_take(
                self.scheduler.id,
                *index,
                data,
                *count as u64,
                self.data_type,
                false,
                take_header(&self.header_type, header.as_deref()),
            )
            .map_err(|_| ())?;
            self.scheduler.send_synced(&lock, messages);

            if !self.scheduler.is_connected() {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// A transfer's header as it goes on the wire: its description and bytes,
/// or `None` for a state without a header type.
type TakeHeader<'a> = Option<(HeaderBytes, &'a [u8])>;

fn take_header<'a>(header_type: &HeaderType, header: Option<&'a [u8]>) -> TakeHeader<'a> {
    let header = header?;
    Some((header_type.bytes(header)?, header))
}

#[allow(clippy::too_many_arguments)]
fn pack_data_take(
    id: u64,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    header: TakeHeader,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_data_take_chunked(
        id,
        data,
        data_count,
        data_type,
        blocking,
        header,
        TRANSFER_CHUNK_SIZE,
    )
}

/// Packs a one-shot transfer; `wrap` serializes its header for the state.
/// The typed header goes with the message that completes the transfer, in
/// front of its elements.
fn pack_take(
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    header: TakeHeader,
    chunk_limit: usize,
    wrap: impl Fn(DataTakeHeader) -> Result<FastVec<32>, String>,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let (description, header_bytes) = match header {
        Some((description, bytes)) => (Some(description), bytes),
        None => (None, &[][..]),
    };
    chunk_transfer(
        data.len(),
        data_type.item_size(),
        chunk_limit,
        |chunk, range| {
            let len = range.len() as u32;
            let (message, header_bytes) = match chunk {
                Chunk::All => (
                    DataTakeHeader::All(data_type, data_count, len, description),
                    header_bytes,
                ),
                Chunk::Start => (DataTakeHeader::StartBatch(data_count, len), &[][..]),
                Chunk::Batch => (DataTakeHeader::Batch(len), &[][..]),
                Chunk::End => (
                    DataTakeHeader::End(data_type, data_count, len, description),
                    header_bytes,
                ),
            };
            let mut message = wrap(message)?;
            message.reserve_exact(header_bytes.len() + range.len());
            message.extend_from_slice(header_bytes);
            message.extend_from_slice(&data[range]);
            Ok(message)
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::server_core) fn pack_data_take_chunked(
    id: u64,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    header: TakeHeader,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_take(data, data_count, data_type, header, chunk_limit, |header| {
        header
            .serialize(id, blocking, true)
            .map_err(|_| "Failed to serialize DataTake header".to_string())
    })
}

#[allow(clippy::too_many_arguments)]
fn pack_data_multi_take(
    id: u64,
    index: u32,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    header: TakeHeader,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_data_multi_take_chunked(
        id,
        index,
        data,
        data_count,
        data_type,
        blocking,
        header,
        TRANSFER_CHUNK_SIZE,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::server_core) fn pack_data_multi_take_chunked(
    id: u64,
    index: u32,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    header: TakeHeader,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_take(data, data_count, data_type, header, chunk_limit, |header| {
        DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
            .map_err(|_| "Failed to serialize DataMultiTake header".to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use super::*;

    fn holder(data: &[u8]) -> DataSlice<'_> {
        DataSlice {
            bytes: data,
            data_type: DataType::U8,
            header: None,
        }
    }

    fn count(receiver: &mut crate::server_core::net::sender::MessageReceiver) -> usize {
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

    /// Runs `f` on another thread and checks that it blocks.
    fn blocked<T: Send + 'static>(
        f: impl FnOnce() -> T + Send + 'static,
    ) -> std::sync::mpsc::Receiver<T> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(f()).unwrap());
        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        receiver
    }

    fn finished<T>(done: &std::sync::mpsc::Receiver<T>) -> T {
        done.recv_timeout(Duration::from_secs(2))
            .expect("blocked producer did not finish")
    }

    fn take(
        connected: bool,
    ) -> (
        Arc<DataTake>,
        Arc<AtomicBool>,
        crate::server_core::net::sender::MessageReceiver,
    ) {
        let (sender, receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(connected));
        let take = DataTake::new(
            "root.take".into(),
            5,
            DataType::U8,
            None,
            sender,
            connected.clone(),
        );
        (take, connected, receiver)
    }

    fn multi(
        connected: bool,
    ) -> (
        Arc<DataMultiTake>,
        Arc<AtomicBool>,
        crate::server_core::net::sender::MessageReceiver,
    ) {
        let (sender, receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(connected));
        let take = DataMultiTake::new(
            "root.take".into(),
            5,
            DataType::U8,
            None,
            sender,
            connected.clone(),
        );
        (take, connected, receiver)
    }

    /// Every transfer waits for the acknowledgement of the one before, which
    /// the client sends on arrival for a non-blocking one.
    #[test]
    fn a_take_waits_for_the_acknowledgement_of_the_previous_one() {
        if crate::test_support::isolated() {
            return;
        }
        let (take, _, mut receiver) = take(true);
        take.sync().unwrap();
        assert!(take.is_idle());
        for blocking in [false, true] {
            take.set(holder(&[1]), blocking, false, false).unwrap();
            assert_eq!(count(&mut receiver), 1);
            assert!(!take.is_idle());

            let producer = take.clone();
            let done = blocked(move || producer.set(holder(&[2]), false, false, false));
            take.acknowledge();
            finished(&done).unwrap();
            assert_eq!(count(&mut receiver), 1);
            take.acknowledge();
            assert!(take.is_idle());
        }

        // With `wait`, the call returns once its own transfer is
        // acknowledged.
        let producer = take.clone();
        let done = blocked(move || producer.set(holder(&[3]), true, false, true));
        assert_eq!(count(&mut receiver), 1);
        take.acknowledge();
        finished(&done).unwrap();
        assert!(take.is_idle());
    }

    /// With no client connected nothing waits, also for a blocking take; the
    /// value is dropped, or cached for the next client.
    #[test]
    fn without_a_client_a_take_is_only_cached_and_nothing_waits() {
        let (take, connected, mut receiver) = take(false);
        for _ in 0..2 {
            take.set(holder(&[1]), true, false, true).unwrap();
        }
        connected.store(true, Ordering::Release);
        take.sync().unwrap();
        assert_eq!(count(&mut receiver), 0, "an uncached take was sent");

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*take);
        take.set(holder(&[2]), true, true, true).unwrap();
        connected.store(true, Ordering::Release);
        take.sync().unwrap();
        assert_eq!(count(&mut receiver), 1, "the cache was not sent");
    }

    /// A cached take set in the reconnect window used to be queued by the
    /// producer and again by the synchronization from the cache (D8).
    #[test]
    fn a_cached_take_in_the_reconnect_window_is_delivered_once() {
        if crate::test_support::isolated() {
            return;
        }
        let (take, connected, mut receiver) = take(true);
        take.sync().unwrap();
        take.set(holder(&[1]), false, true, false).unwrap();
        assert_eq!(count(&mut receiver), 1);

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*take);
        connected.store(true, Ordering::Release);
        take.set(holder(&[2]), true, true, false).unwrap();
        assert_eq!(count(&mut receiver), 0, "a window take was sent");
        take.sync().unwrap();
        assert_eq!(
            count(&mut receiver),
            1,
            "the synchronization sends the cache"
        );

        // The next take waits for the cache's acknowledgement.
        let first = take.clone();
        let done = blocked(move || first.set(holder(&[3]), true, true, false));
        take.acknowledge();
        finished(&done).unwrap();
        assert_eq!(count(&mut receiver), 1);
    }

    /// A blocking take whose wait spans a disconnect is dropped by its
    /// producer; with `cache`, the synchronization delivers it.
    #[test]
    fn a_take_whose_wait_spans_a_reconnect_reaches_the_new_connection_from_the_cache() {
        if crate::test_support::isolated() {
            return;
        }
        let (take, connected, mut receiver) = take(true);
        take.sync().unwrap();
        take.set(holder(&[1]), true, true, false).unwrap();
        assert_eq!(count(&mut receiver), 1);

        let producer = take.clone();
        let done = blocked(move || producer.set(holder(&[2]), true, true, false));
        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*take);
        finished(&done).unwrap();
        assert_eq!(count(&mut receiver), 0, "a stale take was sent");
        connected.store(true, Ordering::Release);
        take.sync().unwrap();
        assert_eq!(count(&mut receiver), 1, "the new value was not sent");
    }

    #[test]
    fn a_cached_multi_take_in_the_reconnect_window_is_delivered_once() {
        if crate::test_support::isolated() {
            return;
        }
        let (take, connected, mut receiver) = multi(false);
        connected.store(true, Ordering::Release);
        take.sync().unwrap();
        assert_eq!(count(&mut receiver), 1, "empty synchronization resets");
        take.set(1, holder(&[1]), false, true, false).unwrap();
        assert_eq!(count(&mut receiver), 1);

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*take);
        connected.store(true, Ordering::Release);
        take.set(1, holder(&[2]), true, true, false).unwrap();
        take.remove_index(3).unwrap();
        assert_eq!(count(&mut receiver), 0, "a window change was sent");
        take.sync().unwrap();
        assert_eq!(
            count(&mut receiver),
            2,
            "the synchronization resets and sends the cache"
        );

        let first = take.clone();
        let done = blocked(move || first.set(1, holder(&[3]), true, true, false));
        take.acknowledge_key(1);
        finished(&done).unwrap();
        assert_eq!(count(&mut receiver), 1);
    }

    #[test]
    fn a_multi_take_whose_wait_spans_a_reconnect_reaches_the_new_connection_from_the_cache() {
        if crate::test_support::isolated() {
            return;
        }
        let (take, connected, mut receiver) = multi(true);
        take.sync().unwrap();
        take.set(1, holder(&[1]), true, true, false).unwrap();
        assert_eq!(count(&mut receiver), 2, "the reset and the take");

        let producer = take.clone();
        let done = blocked(move || producer.set(1, holder(&[2]), true, true, false));
        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*take);
        finished(&done).unwrap();
        connected.store(true, Ordering::Release);
        take.sync().unwrap();
        assert_eq!(count(&mut receiver), 2, "the reset and the new value");
    }

    /// A blocking take of one key does not hold up the others, and a remove
    /// or reset does not wait for it to be taken.
    #[test]
    fn the_keys_of_a_multi_take_do_not_wait_for_each_other() {
        if crate::test_support::isolated() {
            return;
        }
        let (take, _, mut receiver) = multi(true);
        take.sync().unwrap();
        count(&mut receiver);
        take.set(1, holder(&[1]), true, false, false).unwrap();
        take.set(2, holder(&[2]), false, false, false).unwrap();
        assert_eq!(count(&mut receiver), 2);
        assert!(!take.is_index_idle(1));
        take.acknowledge_key(2);
        assert!(take.is_index_idle(2));
        assert!(!take.is_idle());

        let producer = take.clone();
        let done = blocked(move || producer.set(1, holder(&[3]), false, false, false));
        within("remove of a key in flight", {
            let take = take.clone();
            move || take.remove_index(1)
        })
        .unwrap();
        within("reset while a key is in flight", {
            let take = take.clone();
            move || take.reset()
        })
        .unwrap();
        assert_eq!(count(&mut receiver), 2, "the remove and the reset");
        // The client acknowledges the removed blocking value.
        take.acknowledge_key(1);
        finished(&done).unwrap();
        assert_eq!(count(&mut receiver), 1);
    }

    /// A cached set that waited for its key through a removal or a reset is
    /// sent after it, and stays cached: the cache changes with the wire, not
    /// before the wait.
    #[test]
    fn a_multi_take_that_waited_through_a_removal_stays_cached() {
        if crate::test_support::isolated() {
            return;
        }
        let cached = |take: &DataMultiTake| {
            let cache = take.cache.lock();
            cache.get(&1).map(|(data, ..)| data.clone())
        };
        for reset in [false, true] {
            let (take, _, mut receiver) = multi(true);
            take.sync().unwrap();
            count(&mut receiver);
            take.set(1, holder(&[1]), true, true, false).unwrap();
            assert_eq!(cached(&take), Some(vec![1]));

            let producer = take.clone();
            let done = blocked(move || producer.set(1, holder(&[2]), true, true, false));
            assert_eq!(
                cached(&take),
                Some(vec![1]),
                "the waiting set is not stored"
            );
            match reset {
                false => take.remove_index(1).unwrap(),
                true => take.reset().unwrap(),
            }
            assert_eq!(cached(&take), None);
            // The client acknowledges the removed blocking value.
            take.acknowledge_key(1);
            finished(&done).unwrap();
            assert_eq!(count(&mut receiver), 3, "both takes and the removal");
            assert_eq!(cached(&take), Some(vec![2]));
        }
    }

    /// Keys that come and go leave nothing behind in the scheduler.
    #[test]
    fn a_multi_take_keeps_its_slots_only_for_keys_in_flight() {
        let (take, _, mut receiver) = multi(true);
        take.sync().unwrap();
        for key in 0..100 {
            take.set(key, holder(&[1]), false, false, false).unwrap();
            take.acknowledge_key(key);
            take.remove_index(key).unwrap();
        }
        count(&mut receiver);
        assert_eq!(take.scheduler.slot_count(), 0);
    }
}

#[cfg(all(test, feature = "client"))]
mod transfer_tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::client::messages::{MessageSender as ClientSender, MessagesParser};
    use crate::client::states::data_take::{
        DataMultiTake as ClientMulti, DataTake as ClientTake, UpdateDataMultiTake, UpdateDataTake,
    };
    use crate::protocol::ServerHeader;

    /// Applies the queued server messages of the multi take `61` to `client`.
    fn apply(
        receiver: &mut crate::server_core::net::sender::MessageReceiver,
        client: &ClientMulti<u8>,
    ) {
        while let Ok(Some((message, _))) = receiver.try_recv() {
            let (mut parser, (header, payload)) =
                MessagesParser::from_bytes(message.into_bytes()).unwrap();
            match header {
                ServerHeader::DataMultiTake(61, DataMultiTakeHeader::Reset) => client.reset(),
                ServerHeader::DataMultiTake(
                    61,
                    DataMultiTakeHeader::Modify(key, header, blocking),
                ) => {
                    client.update(key, (header, payload), blocking).unwrap();
                }
                _ => panic!("unexpected server message"),
            }
            assert!(parser.next().unwrap().is_none());
        }
    }

    /// A resynchronization used to reset the client only when no key was
    /// cached, so a value the client had not taken survived a reconnect
    /// whenever another key was cached.
    #[test]
    fn a_resync_leaves_the_client_with_the_cached_keys() {
        let (client_sender, _acks) = ClientSender::new();
        let client = ClientMulti::<u8>::new("takes".into(), 61, client_sender);
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(true));
        let server = DataMultiTake::new(
            "takes".into(),
            61,
            DataType::U8,
            None,
            sender,
            connected.clone(),
        );
        let data = |bytes: &'static [u8]| DataSlice {
            bytes,
            data_type: DataType::U8,
            header: None,
        };

        server.sync().unwrap();
        server.set(1, data(&[1]), false, true, false).unwrap();
        server.set(2, data(&[2]), false, false, false).unwrap();
        apply(&mut receiver, &client);
        assert!(client.is_some(1) && client.is_some(2));

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*server);
        client.disconnected();
        connected.store(true, Ordering::Release);
        server.sync().unwrap();
        apply(&mut receiver, &client);

        assert_eq!(client.take(1).as_deref(), Some(&[1u8][..]));
        assert!(!client.is_some(2), "an uncached value survived the resync");
    }

    #[test]
    fn real_take_chunks_publish_once_and_ack_on_arrival_or_consumption() {
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
                                None,
                            ),
                            Some(key) => pack_data_multi_take(
                                61,
                                key,
                                &expected,
                                size as u64,
                                DataType::U8,
                                blocking,
                                None,
                            ),
                        }
                        .unwrap();
                        let total = messages.len();
                        assert_eq!(total, size.div_ceil(TRANSFER_CHUNK_SIZE));
                        for (n, (message, _)) in messages.into_iter().enumerate() {
                            let (mut parser, (header, payload)) =
                                MessagesParser::from_bytes(message.into_bytes()).unwrap();
                            assert_eq!(header.changes_state(), n + 1 == total);
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
                            if n + 1 == total && !blocking {
                                // A non-blocking transfer is acknowledged
                                // when it arrives.
                                crate::test_support::ack(&mut receiver, 61);
                            }
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
