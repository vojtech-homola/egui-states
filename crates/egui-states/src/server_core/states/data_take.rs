use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::event::Event;
use crate::hashing::NoHashMap;
use crate::protocol::data::{DataMultiTakeHeader, DataTakeHeader, DataType, HeaderBytes};
use crate::protocol::{FastVec, TRANSFER_CHUNK_SIZE};
use crate::server_core::error::CoreError;
use crate::server_core::net::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::states::data::{DataSlice, HeaderType, SyncState, check_data_type};
use crate::server_core::transfer::{Chunk, chunk_transfer};

// DataTake --------------------------------------------------
/// A cached transfer: its elements, their count and its encoded header.
type Cached = (Vec<u8>, usize, Option<Vec<u8>>);

struct TakeState {
    cache: Option<Cached>,
    sync: SyncState,
}

/// Server side of a one-shot numeric transfer.
///
/// A producer waits for the client to take a pending blocking value before it
/// takes the state lock, so a remove or reset can still cancel that value.
/// It then stores the cache and queues its chunks under the lock. While the
/// state awaits a synchronization with a newly connected client, it only
/// stores: the synchronization sends the cache, and sending as well would
/// deliver the value twice. When a synchronization ran after the producer
/// checked the connection, it sent the previous cache, so the producer starts
/// over for the new connection; see [`finish`].
pub(crate) struct DataTake {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    header_type: HeaderType,
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
        header_type: Option<u32>,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            data_type,
            item_size: data_type.item_size(),
            header_type: HeaderType::new(header_type),
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
        data: DataSlice,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        self.header_type
            .require(&self.name, header, "with every take")?;
        let slice = data.bytes;

        loop {
            let generation = self.state.lock().sync.generation();
            let messages = if self.connected.load(Ordering::Acquire) {
                let messages = pack_data_take(
                    self.id,
                    slice,
                    elements as u64,
                    self.data_type,
                    blocking,
                    update,
                    self.take_header(header),
                )?;
                wait_for_slot(&self.event, blocking);
                Some(messages)
            } else {
                None
            };

            let mut state = self.state.lock();
            let messages = match finish(
                &state.sync,
                &self.event,
                &self.connected,
                generation,
                blocking,
                messages,
            ) {
                Finish::Retry => continue,
                Finish::Store => None,
                Finish::Send(messages) => Some(messages),
            };
            state.cache = cache.then(|| (slice.to_vec(), elements, header.map(<[u8]>::to_vec)));
            for (message, single) in messages.into_iter().flatten() {
                self.sender.send_set(message, single);
            }
            return Ok(());
        }
    }
}

impl DataTake {
    fn take_header<'a>(&self, header: Option<&'a [u8]>) -> TakeHeader<'a> {
        take_header(&self.header_type, header)
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

        if let Some((ref data, count, ref header)) = state.cache {
            let messages = pack_data_take(
                self.id,
                data,
                count as u64,
                self.data_type,
                false,
                false,
                self.take_header(header.as_deref()),
            )
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
    values: NoHashMap<u32, Cached>,
    sync: SyncState,
}

/// One-shot numeric transfers indexed by key; see [`DataTake`] for the
/// producer protocol. All keys share one acknowledgement stream.
pub(crate) struct DataMultiTake {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    header_type: HeaderType,
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
        header_type: Option<u32>,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            data_type,
            item_size: data_type.item_size(),
            header_type: HeaderType::new(header_type),
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
        data: DataSlice,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        self.header_type
            .require(&self.name, header, "with every take")?;
        let slice = data.bytes;

        loop {
            let generation = self.state.read().sync.generation();
            let messages = if self.connected.load(Ordering::Acquire) {
                let messages = pack_data_multi_take(
                    self.id,
                    index,
                    slice,
                    elements as u64,
                    self.data_type,
                    blocking,
                    update,
                    take_header(&self.header_type, header),
                )?;
                wait_for_slot(&self.event, blocking);
                Some(messages)
            } else {
                None
            };

            // Held until every chunk is queued, so concurrent non-blocking
            // transfers cannot interleave their chunks on the wire.
            let mut w = self.state.write();
            let messages = match finish(
                &w.sync,
                &self.event,
                &self.connected,
                generation,
                blocking,
                messages,
            ) {
                Finish::Retry => continue,
                Finish::Store => None,
                Finish::Send(messages) => Some(messages),
            };
            if cache {
                let cached = (slice.to_vec(), elements, header.map(<[u8]>::to_vec));
                w.values.insert(index, cached);
            } else {
                w.values.remove(&index);
            }
            for (message, single) in messages.into_iter().flatten() {
                self.sender.send_set(message, single);
            }
            return Ok(());
        }
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

        // The client keeps the values it has not taken across a disconnect;
        // drop them, so it holds exactly the cached keys, like a DataMulti.
        let message = DataMultiTakeHeader::Reset(false).serialize(self.id)?;
        self.sender.send(message);
        for (index, (data, count, header)) in w.values.iter() {
            let messages = pack_data_multi_take(
                self.id,
                *index,
                data,
                *count as u64,
                self.data_type,
                false,
                false,
                take_header(&self.header_type, header.as_deref()),
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

/// Waits until no blocking transfer is pending. A blocking send takes the
/// slot, so the next send waits until the client took this one; a
/// non-blocking send leaves it free.
fn wait_for_slot(event: &Event, blocking: bool) {
    match blocking {
        true => event.wait_clear(),
        false => event.wait(),
    }
}

/// What a producer does with its transfer once it holds the state lock.
enum Finish {
    /// Store the value and queue these messages.
    Send(Vec<(FastVec<32>, bool)>),
    /// Only store the value: no client is synchronized, and the next
    /// synchronization sends the cache.
    Store,
    /// Start over: a synchronization ran after the producer checked the
    /// connection, so it sent the previous cache and the transfer packed for
    /// the old connection, if any, is stale.
    Retry,
}

/// Decides, under the caller's state lock, how a producer finishes the
/// transfer it packed (`Some`) or skipped (`None`) after it read `generation`
/// and checked the connection. A blocking producer that does not send returns
/// the slot its wait consumed.
fn finish(
    sync: &SyncState,
    event: &Event,
    connected: &AtomicBool,
    generation: u64,
    blocking: bool,
    messages: Option<Vec<(FastVec<32>, bool)>>,
) -> Finish {
    let packed = messages.is_some();
    let finish = match (sync.sendable(connected), messages) {
        (Some(current), Some(messages)) if current == generation => return Finish::Send(messages),
        (None, _) => Finish::Store,
        (Some(_), _) => Finish::Retry,
    };
    if blocking && packed {
        event.set();
    }
    finish
}

#[allow(clippy::too_many_arguments)]
fn pack_data_take(
    id: u64,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
    header: TakeHeader,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_data_take_chunked(
        id,
        data,
        data_count,
        data_type,
        blocking,
        update,
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
    update: bool,
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
                    DataTakeHeader::All(data_type, data_count, update, len, description),
                    header_bytes,
                ),
                Chunk::Start => (DataTakeHeader::StartBatch(data_count, len), &[][..]),
                Chunk::Batch => (DataTakeHeader::Batch(len), &[][..]),
                Chunk::End => (
                    DataTakeHeader::End(data_type, data_count, update, len, description),
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
    update: bool,
    header: TakeHeader,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_take(
        data,
        data_count,
        data_type,
        update,
        header,
        chunk_limit,
        |header| {
            header
                .serialize(id, blocking, true)
                .map_err(|_| "Failed to serialize DataTake header".to_string())
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn pack_data_multi_take(
    id: u64,
    index: u32,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
    header: TakeHeader,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_data_multi_take_chunked(
        id,
        index,
        data,
        data_count,
        data_type,
        blocking,
        update,
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
    update: bool,
    header: TakeHeader,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_take(
        data,
        data_count,
        data_type,
        update,
        header,
        chunk_limit,
        |header| {
            DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
                .map_err(|_| "Failed to serialize DataMultiTake header".to_string())
        },
    )
}

#[cfg(test)]
mod tests {
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
            None,
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
            None,
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
            2,
            "the synchronization resets and sends the cache"
        );

        let first = take.clone();
        within("first blocking multi take after sync", move || {
            first.set(1, holder(&[3]), true, false, true)
        })
        .unwrap();
        assert_eq!(count(&mut receiver), 1);
    }

    /// A synchronization that ran after the producer checked the connection
    /// sent the previous cache, so the producer must start over instead of
    /// only storing; a blocking producer returns the slot it consumed.
    #[test]
    fn finish_retries_after_a_synchronization_it_did_not_see() {
        let event = Event::new();
        let connected = AtomicBool::new(true);
        let mut sync = SyncState::new();
        let packed = || Some(vec![(FastVec::new(), false)]);

        // Awaiting a synchronization: only store, and free the slot.
        assert!(matches!(
            finish(&sync, &event, &connected, 0, true, packed()),
            Finish::Store
        ));
        assert!(event.is_set());
        event.clear();

        sync.synchronized();
        assert!(matches!(
            finish(&sync, &event, &connected, 0, true, packed()),
            Finish::Send(_)
        ));
        assert!(!event.is_set(), "a sent blocking transfer keeps the slot");
        // Not connected at the check, synchronized now.
        assert!(matches!(
            finish(&sync, &event, &connected, 0, true, None),
            Finish::Retry
        ));
        assert!(!event.is_set(), "a producer that did not wait has no slot");

        // A reset and a resynchronization while the producer waited.
        sync.reset();
        sync.synchronized();
        assert!(matches!(
            finish(&sync, &event, &connected, 0, true, packed()),
            Finish::Retry
        ));
        assert!(event.is_set());
        event.clear();

        connected.store(false, Ordering::Release);
        assert!(matches!(
            finish(&sync, &event, &connected, 1, false, packed()),
            Finish::Store
        ));
        assert!(!event.is_set(), "a non-blocking producer consumed no slot");
    }

    /// Releases a producer waiting for the slot with a reset and finishes a
    /// resynchronization before the producer can take the state lock.
    fn reconnect_under_lock(sync: &mut SyncState, event: &Event, connected: &AtomicBool) {
        connected.store(false, Ordering::Release);
        sync.reset();
        event.set();
        connected.store(true, Ordering::Release);
        sync.synchronized();
    }

    /// A blocking take whose wait spanned a reset and a resynchronization
    /// used to be only cached, after the synchronization had sent the old
    /// cache, so the new connection never received it.
    #[test]
    fn a_take_whose_wait_spans_a_resync_reaches_the_new_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(true));
        let take = DataTake::new(
            "root.take".into(),
            5,
            DataType::U8,
            None,
            sender,
            connected.clone(),
        );
        take.sync().unwrap();
        take.set(holder(&[1]), true, false, true).unwrap();
        assert_eq!(count(&mut receiver), 1);

        let producer = take.clone();
        let waiting = std::thread::spawn(move || producer.set(holder(&[2]), true, false, true));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(count(&mut receiver), 0, "a blocking take did not wait");

        reconnect_under_lock(&mut take.state.lock().sync, &take.event, &connected);
        within("take after the resync", move || waiting.join().unwrap()).unwrap();
        assert_eq!(count(&mut receiver), 1, "the new value was not sent");
    }

    #[test]
    fn a_multi_take_whose_wait_spans_a_resync_reaches_the_new_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(true));
        let take = DataMultiTake::new(
            "root.take".into(),
            5,
            DataType::U8,
            None,
            sender,
            connected.clone(),
        );
        take.sync().unwrap();
        take.set(1, holder(&[1]), true, false, true).unwrap();
        assert_eq!(count(&mut receiver), 2, "the reset and the take");

        let producer = take.clone();
        let waiting = std::thread::spawn(move || producer.set(1, holder(&[2]), true, false, true));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(count(&mut receiver), 0, "a blocking take did not wait");

        reconnect_under_lock(&mut take.state.write().sync, &take.event, &connected);
        within("multi take after the resync", move || {
            waiting.join().unwrap()
        })
        .unwrap();
        assert_eq!(count(&mut receiver), 1, "the new value was not sent");
    }
}

#[cfg(all(test, feature = "client"))]
mod transfer_tests {
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
                ServerHeader::DataMultiTake(61, DataMultiTakeHeader::Reset(_)) => client.reset(),
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
        server.set(1, data(&[1]), false, false, true).unwrap();
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
                                None,
                            ),
                            Some(key) => pack_data_multi_take(
                                61,
                                key,
                                &expected,
                                size as u64,
                                DataType::U8,
                                blocking,
                                true,
                                None,
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
