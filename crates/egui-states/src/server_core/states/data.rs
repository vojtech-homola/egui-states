use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::event::Event;
use crate::hashing::NoHashMap;
use crate::protocol::data::{
    DataHeader, DataType, HeaderBytes, HeaderUpdate, MultiDataHeader, TransportType,
};
use crate::protocol::{FastVec, TRANSFER_CHUNK_SIZE, check_value_size};
use crate::server_core::error::CoreError;
use crate::server_core::net::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::transfer::{Chunk, chunk_transfer};

/// The input of a Data operation, as the front-ends hand it to the cores,
/// which copy what they keep: native-endian elements and, optionally, a new
/// encoded header.
#[derive(Clone, Copy)]
pub(crate) struct DataSlice<'a> {
    pub bytes: &'a [u8],
    pub data_type: DataType,
    pub header: Option<&'a [u8]>,
}

/// The header type of a Data state, if it was registered with one, and the
/// checks every header given to it passes.
#[derive(Clone, Copy)]
pub(crate) struct HeaderType(Option<u32>);

impl HeaderType {
    pub(crate) fn new(type_id: Option<u32>) -> Self {
        Self(type_id)
    }

    /// Checks a header given to state `name`: the state must have a header
    /// type and the header must fit the value size limit.
    pub(crate) fn check<'a>(
        &self,
        name: &str,
        header: Option<&'a [u8]>,
    ) -> Result<Option<&'a [u8]>, String> {
        let Some(header) = header else {
            return Ok(None);
        };
        if self.0.is_none() {
            return Err(format!("Data '{name}' was registered without a header"));
        }
        check_value_size(format_args!("{name} header"), header.len())?;
        Ok(Some(header))
    }

    /// Checks that a change that creates data carries a header: a state with
    /// a header type never holds data without one. `what` completes the
    /// error message, such as "to create its data".
    pub(crate) fn require(
        &self,
        name: &str,
        header: Option<&[u8]>,
        what: impl fmt::Display,
    ) -> Result<(), CoreError> {
        if self.0.is_some() && header.is_none() {
            return Err(CoreError::invalid_argument(format!(
                "Data '{name}' needs a header {what}"
            )));
        }
        Ok(())
    }

    /// The wire form of a checked header, `Keep` without one. A
    /// synchronization gives the stored header, so a state with a header
    /// type always says what it is.
    pub(crate) fn change<'a>(&self, header: Option<&'a [u8]>) -> WireHeader<'a> {
        match (self.0, header) {
            (Some(type_id), Some(bytes)) => WireHeader::set(type_id, bytes),
            _ => WireHeader::KEEP,
        }
    }

    /// The wire description of a checked header.
    pub(crate) fn bytes(&self, header: &[u8]) -> Option<HeaderBytes> {
        self.0.map(|type_id| HeaderBytes {
            type_id,
            size: header.len() as u32,
        })
    }
}

/// A header change as it goes on the wire: the update and the bytes it puts
/// in front of the elements.
#[derive(Clone, Copy)]
pub(crate) struct WireHeader<'a> {
    pub(crate) update: HeaderUpdate,
    pub(crate) bytes: &'a [u8],
}

impl<'a> WireHeader<'a> {
    pub(crate) const KEEP: Self = Self {
        update: HeaderUpdate::Keep,
        bytes: &[],
    };

    pub(crate) fn set(type_id: u32, bytes: &'a [u8]) -> Self {
        Self {
            update: HeaderUpdate::Set(HeaderBytes {
                type_id,
                size: bytes.len() as u32,
            }),
            bytes,
        }
    }

    /// Serializes `header`, which carries this update, followed by the
    /// header bytes.
    fn message(
        &self,
        header: impl FnOnce() -> Result<FastVec<32>, ()>,
    ) -> Result<FastVec<32>, String> {
        let mut message = header().map_err(|_| "Failed to serialize header".to_string())?;
        message.extend_from_slice(self.bytes);
        Ok(message)
    }
}

/// The data of a [`Data`] or of one [`DataMulti`] key: its elements and its
/// encoded header, which stays empty for a state without a header type.
struct Stored {
    bytes: Vec<u8>,
    count: usize,
    header: Vec<u8>,
}

impl Stored {
    fn new(bytes: &[u8], count: usize, header: Option<&[u8]>) -> Self {
        Self {
            bytes: bytes.to_vec(),
            count,
            header: header.map(<[u8]>::to_vec).unwrap_or_default(),
        }
    }

    /// Replaces the elements, and the header when one is given.
    fn set(&mut self, bytes: &[u8], count: usize, header: Option<&[u8]>) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
        self.count = count;
        self.store_header(header);
    }

    fn store_header(&mut self, header: Option<&[u8]>) {
        if let Some(header) = header {
            self.header.clear();
            self.header.extend_from_slice(header);
        }
    }
}

/// Connection bookkeeping a state keeps under its storage lock.
///
/// A producer stores its change and, under the same lock, asks whether it may
/// send it. Until the state is synchronized with a newly connected client it
/// may not: the synchronization sends the stored data, so a delta sent as well
/// would be applied twice. A reset bumps the generation, so a producer that
/// was waiting for an acknowledgement of a previous connection drops its work.
pub(crate) struct SyncState {
    required: bool,
    generation: u64,
}

impl SyncState {
    pub(crate) fn new() -> Self {
        Self {
            required: true,
            generation: 0,
        }
    }

    /// The generation to send in, or `None` if the change must only be stored.
    pub(crate) fn sendable(&self, connected: &AtomicBool) -> Option<u64> {
        (!self.required && connected.load(Ordering::Acquire)).then_some(self.generation)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the state awaits a synchronization with a newly connected
    /// client.
    pub(crate) fn required(&self) -> bool {
        self.required
    }

    pub(crate) fn synchronized(&mut self) {
        self.required = false;
    }

    pub(crate) fn reset(&mut self) {
        self.required = true;
        self.generation = self.generation.wrapping_add(1);
    }
}

// Data --------------------------------------------------
struct DataStorage {
    /// `None` while the data is not set.
    value: Option<Stored>,
    sync: SyncState,
}

/// Server storage of one numeric buffer and its header.
///
/// The data starts "not set". Only [`Data::set`] creates it, with a header
/// when the state has a header type; [`Data::reset`] returns it to "not set".
/// Every other change needs data that is set.
///
/// Producers are serialized by `producer`, which they hold across the wait
/// for the client's acknowledgement of the previous transfer, so the stored
/// data changes in the order the client receives the changes. The storage
/// lock is held only to change or read the bytes; nothing that waits holds
/// it, because a disconnect reset takes it from the connection's writer.
pub(crate) struct Data {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    header_type: HeaderType,
    producer: Mutex<()>,
    storage: RwLock<DataStorage>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl Data {
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
            producer: Mutex::new(()),
            storage: RwLock::new(DataStorage {
                value: None,
                sync: SyncState::new(),
            }),
            sender,
            connected,
            event: Event::new(),
        })
    }

    fn not_set(&self) -> CoreError {
        CoreError::not_found(format!("Data '{}' is not set", self.name))
    }

    /// Replaces the buffer, creating the data when it is not set; creating
    /// the data of a state with a header type needs a header.
    pub(crate) fn set(&self, data: DataSlice, update: bool) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        let slice = data.bytes;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            match w.value.as_mut() {
                Some(value) => value.set(slice, elements, header),
                None => {
                    self.header_type
                        .require(&self.name, header, "to create its data")?;
                    w.value = Some(Stored::new(slice, elements, header));
                }
            }
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let count = elements as u64;
        let messages = pack_data(
            self.id,
            slice,
            TransportType::Set(count),
            count,
            self.data_type,
            None,
            update,
            self.header_type.change(header),
        )?;
        self.send_after_ack(generation, messages);
        Ok(())
    }

    pub(crate) fn add(&self, data: DataSlice, update: bool) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        let slice = data.bytes;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            let value = w.value.as_mut().ok_or_else(|| self.not_set())?;
            value.bytes.extend_from_slice(slice);
            value.count += elements;
            value.store_header(header);
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let count = elements as u64;
        let messages = pack_data(
            self.id,
            slice,
            TransportType::Add(count),
            count,
            self.data_type,
            None,
            update,
            self.header_type.change(header),
        )?;
        self.send_after_ack(generation, messages);
        Ok(())
    }

    pub(crate) fn replace(
        &self,
        data: DataSlice,
        index: usize,
        update: bool,
    ) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        let slice = data.bytes;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            let value = w.value.as_mut().ok_or_else(|| self.not_set())?;
            if index
                .checked_add(elements)
                .is_none_or(|end| end > value.count)
            {
                return Err(format!(
                    "Replace range out of bounds: index {} + data count {} exceeds current size {}",
                    index, elements, value.count
                )
                .into());
            }
            let byte_index = index * self.item_size;
            value.bytes[byte_index..byte_index + slice.len()].copy_from_slice(slice);
            value.store_header(header);
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let count = elements as u64;
        let messages = pack_data(
            self.id,
            slice,
            TransportType::Replace(index as u64, count),
            count,
            self.data_type,
            None,
            update,
            self.header_type.change(header),
        )?;
        self.send_after_ack(generation, messages);
        Ok(())
    }

    pub(crate) fn remove(
        &self,
        index: usize,
        size: usize,
        header: Option<&[u8]>,
        update: bool,
    ) -> Result<(), CoreError> {
        if size == 0 {
            return Err("Invalid remove size: size must be greater than 0".into());
        }
        let header = self.header_type.check(&self.name, header)?;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            let value = w.value.as_mut().ok_or_else(|| self.not_set())?;
            if index.checked_add(size).is_none_or(|end| end > value.count) {
                return Err(format!(
                    "Remove range out of bounds: index {} + size {} exceeds current size {}",
                    index, size, value.count
                )
                .into());
            }
            let byte_index = index * self.item_size;
            value
                .bytes
                .drain(byte_index..byte_index + size * self.item_size);
            value.count -= size;
            value.store_header(header);
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let change = self.header_type.change(header);
        let message = change.message(|| {
            DataHeader::Drain(index as u64, size as u64, update, change.update)
                .serialize(self.id, false)
        })?;
        self.send_after_ack(generation, vec![(message, false)]);
        Ok(())
    }

    /// Empties the buffer; the header stays unless a new one is given.
    pub(crate) fn clear(&self, header: Option<&[u8]>, update: bool) -> Result<(), CoreError> {
        let header = self.header_type.check(&self.name, header)?;
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        let value = w.value.as_mut().ok_or_else(|| self.not_set())?;
        value.bytes.clear();
        value.count = 0;
        value.store_header(header);

        // The client does not acknowledge a clear, so it is queued at once,
        // under the storage lock so a reset cannot slip in before it.
        if w.sync.sendable(&self.connected).is_some() {
            let change = self.header_type.change(header);
            let message = change
                .message(|| DataHeader::Clear(update, change.update).serialize(self.id, false))?;
            self.sender.send(message);
        }

        Ok(())
    }

    /// Replaces the header alone. Like a clear, the client does not
    /// acknowledge it, so it is queued without waiting for an
    /// acknowledgement of its own. The producer mutex still orders it after
    /// a change another producer is sending, so it cannot overtake a stored
    /// header that change carries.
    pub(crate) fn set_header(&self, header: &[u8], update: bool) -> Result<(), CoreError> {
        self.header_type.check(&self.name, Some(header))?;
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        let value = w.value.as_mut().ok_or_else(|| self.not_set())?;
        value.store_header(Some(header));

        if w.sync.sendable(&self.connected).is_some()
            && let Some(bytes) = self.header_type.bytes(header)
        {
            let change = WireHeader::set(bytes.type_id, header);
            let message =
                change.message(|| DataHeader::Header(update, bytes).serialize(self.id, false))?;
            self.sender.send(message);
        }

        Ok(())
    }

    /// Returns the data and its header to "not set"; data that is not set
    /// stays so. Like a clear, it is queued at once.
    pub(crate) fn reset(&self, update: bool) -> Result<(), CoreError> {
        let message = DataHeader::Reset(update)
            .serialize(self.id, false)
            .map_err(|_| "Failed to serialize reset header".to_string())?;
        let previous = {
            let _producer = self.producer.lock();
            let mut w = self.storage.write();
            let previous = w.value.take();
            if previous.is_some() && w.sync.sendable(&self.connected).is_some() {
                self.sender.send(message);
            }
            previous
        };
        // The buffer is freed after both locks are released.
        drop(previous);

        Ok(())
    }

    /// Calls `f` with the bytes, `None` while the data is not set.
    pub(crate) fn get<R>(&self, f: impl FnOnce(Option<&[u8]>) -> R) -> R {
        let storage = self.storage.read();
        f(storage.value.as_ref().map(|value| value.bytes.as_slice()))
    }

    /// Calls `f` with the encoded header, `None` while the data is not set;
    /// a state without a header type has an empty one.
    pub(crate) fn header<R>(&self, f: impl FnOnce(Option<&[u8]>) -> R) -> R {
        let storage = self.storage.read();
        f(storage.value.as_ref().map(|value| value.header.as_slice()))
    }

    /// Calls `f` with the bytes and the encoded header under one read lock,
    /// `None` while the data is not set.
    pub(crate) fn get_with_header<R>(&self, f: impl FnOnce(Option<(&[u8], &[u8])>) -> R) -> R {
        let storage = self.storage.read();
        f(storage
            .value
            .as_ref()
            .map(|value| (value.bytes.as_slice(), value.header.as_slice())))
    }

    fn send_after_ack(&self, generation: u64, messages: Vec<(FastVec<32>, bool)>) {
        send_after_ack(
            &self.storage,
            |storage| &storage.sync,
            &self.event,
            &self.connected,
            &self.sender,
            generation,
            messages,
        );
    }
}

impl Acknowledge for Data {
    fn acknowledge(&self) {
        self.event.set();
    }

    fn reset(&self) {
        // Bump the generation before releasing a waiting producer, so it sees
        // that its work belongs to the old connection.
        self.storage.write().sync.reset();
        self.event.set();
    }
}

impl SyncTrait for Data {
    fn sync(&self) -> Result<(), ()> {
        // The producer mutex keeps a producer that was released by the reset
        // from consuming this synchronization's acknowledgement. No producer
        // of this state waits for an acknowledgement here: until this point
        // the state required a synchronization, so producers only stored.
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        w.sync.synchronized();

        // The client keeps its data across a disconnect, so a synchronization
        // always says what the data is: not set, or a set of its elements,
        // also when there are none, with the stored header.
        match &w.value {
            None => {
                let message = DataHeader::Reset(false)
                    .serialize(self.id, false)
                    .map_err(|_| ())?;
                self.sender.send(message);
                self.event.set();
            }
            Some(value) => {
                let count = value.count as u64;
                let messages = pack_data(
                    self.id,
                    &value.bytes,
                    TransportType::Set(count),
                    count,
                    self.data_type,
                    None,
                    false,
                    self.header_type.change(Some(&value.header)),
                )
                .map_err(|_| ())?;

                self.event.clear();
                for (message, single) in messages {
                    self.sender.send_set(message, single);
                }
            }
        }

        Ok(())
    }
}

// DataMulti --------------------------------------------------
struct MultiStorage {
    values: NoHashMap<u32, Stored>,
    sync: SyncState,
}

fn missing_key(index: u32) -> CoreError {
    CoreError::not_found(format!("DataMulti index {} does not exist", index))
}

/// Server storage of numeric buffers indexed by key, each with its header.
///
/// Only [`DataMulti::set`] creates a key, with a header when the state has a
/// header type; every other change needs an existing key.
///
/// Uses the same producer protocol as [`Data`]; all keys share one stream of
/// acknowledgements.
pub(crate) struct DataMulti {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    header_type: HeaderType,
    producer: Mutex<()>,
    storage: RwLock<MultiStorage>,
    sync_counter: Mutex<usize>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl DataMulti {
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
            producer: Mutex::new(()),
            storage: RwLock::new(MultiStorage {
                values: NoHashMap::default(),
                sync: SyncState::new(),
            }),
            sync_counter: Mutex::new(0),
            sender,
            connected,
            event: Event::new(),
        })
    }

    /// Removes `index` and its header.
    pub(crate) fn remove_index(&self, index: u32, update: bool) -> Result<(), String> {
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        if w.values.remove(&index).is_some() && w.sync.sendable(&self.connected).is_some() {
            let message = MultiDataHeader::Remove(index, update)
                .serialize(self.id)
                .map_err(|_| "Failed to serialize remove index header".to_string())?;
            self.sender.send(message);
        }

        Ok(())
    }

    /// Removes every key and header.
    pub(crate) fn reset(&self, update: bool) -> Result<(), String> {
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        if !w.values.is_empty() {
            w.values.clear();
            if w.sync.sendable(&self.connected).is_some() {
                let message = MultiDataHeader::Reset(update)
                    .serialize(self.id)
                    .map_err(|_| "Failed to serialize reset header".to_string())?;
                self.sender.send(message);
            }
        }

        Ok(())
    }

    /// Sets the buffer at `index`, creating the key; creating a key of a
    /// state with a header type needs a header.
    pub(crate) fn set(&self, index: u32, data: DataSlice, update: bool) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        let slice = data.bytes;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            match w.values.get_mut(&index) {
                Some(value) => value.set(slice, elements, header),
                None => {
                    self.header_type.require(
                        &self.name,
                        header,
                        format_args!("to create index {index}"),
                    )?;
                    w.values.insert(index, Stored::new(slice, elements, header));
                }
            }
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let count = elements as u64;
        let messages = pack_data(
            self.id,
            slice,
            TransportType::Set(count),
            count,
            self.data_type,
            Some(index),
            update,
            self.header_type.change(header),
        )?;
        self.send_after_ack(generation, messages);
        Ok(())
    }

    pub(crate) fn add(&self, index: u32, data: DataSlice, update: bool) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        let slice = data.bytes;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            let value = w.values.get_mut(&index).ok_or_else(|| missing_key(index))?;
            value.bytes.extend_from_slice(slice);
            value.count += elements;
            value.store_header(header);
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let count = elements as u64;
        let messages = pack_data(
            self.id,
            slice,
            TransportType::Add(count),
            count,
            self.data_type,
            Some(index),
            update,
            self.header_type.change(header),
        )?;
        self.send_after_ack(generation, messages);
        Ok(())
    }

    pub(crate) fn replace(
        &self,
        index: u32,
        data_index: usize,
        data: DataSlice,
        update: bool,
    ) -> Result<(), CoreError> {
        let elements = check_data_type(&data, self.data_type, self.item_size)?;
        let header = self.header_type.check(&self.name, data.header)?;
        let slice = data.bytes;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            let value = w.values.get_mut(&index).ok_or_else(|| missing_key(index))?;
            if data_index
                .checked_add(elements)
                .is_none_or(|end| end > value.count)
            {
                return Err(format!(
                    "Replace range out of bounds for DataMulti index {}: data_index {} + data count {} exceeds current size {}",
                    index, data_index, elements, value.count
                ).into());
            }
            let byte_index = data_index * self.item_size;
            value.bytes[byte_index..byte_index + slice.len()].copy_from_slice(slice);
            value.store_header(header);
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let count = elements as u64;
        let messages = pack_data(
            self.id,
            slice,
            TransportType::Replace(data_index as u64, count),
            count,
            self.data_type,
            Some(index),
            update,
            self.header_type.change(header),
        )?;
        self.send_after_ack(generation, messages);
        Ok(())
    }

    pub(crate) fn remove(
        &self,
        index: u32,
        data_index: usize,
        size: usize,
        header: Option<&[u8]>,
        update: bool,
    ) -> Result<(), CoreError> {
        if size == 0 {
            return Err("Invalid remove size: size must be greater than 0".into());
        }
        let header = self.header_type.check(&self.name, header)?;

        let _producer = self.producer.lock();
        let generation = {
            let mut w = self.storage.write();
            let value = w.values.get_mut(&index).ok_or_else(|| missing_key(index))?;
            if data_index
                .checked_add(size)
                .is_none_or(|end| end > value.count)
            {
                return Err(format!(
                    "Remove range out of bounds for DataMulti index {}: data_index {} + size {} exceeds current size {}",
                    index, data_index, size, value.count
                ).into());
            }
            let byte_index = data_index * self.item_size;
            value
                .bytes
                .drain(byte_index..byte_index + size * self.item_size);
            value.count -= size;
            value.store_header(header);
            w.sync.sendable(&self.connected)
        };
        let Some(generation) = generation else {
            return Ok(());
        };

        let change = self.header_type.change(header);
        let message = change.message(|| {
            let header = DataHeader::Drain(data_index as u64, size as u64, update, change.update);
            MultiDataHeader::serialize_modify(self.id, index, header)
        })?;
        self.send_after_ack(generation, vec![(message, false)]);
        Ok(())
    }

    /// Empties the buffer at `index`; its header stays unless a new one is
    /// given.
    pub(crate) fn clear(
        &self,
        index: u32,
        header: Option<&[u8]>,
        update: bool,
    ) -> Result<(), CoreError> {
        let header = self.header_type.check(&self.name, header)?;
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        let value = w.values.get_mut(&index).ok_or_else(|| missing_key(index))?;
        value.bytes.clear();
        value.count = 0;
        value.store_header(header);

        // Not acknowledged by the client: queued at once, under the storage lock.
        if w.sync.sendable(&self.connected).is_some() {
            let change = self.header_type.change(header);
            let message = change.message(|| {
                MultiDataHeader::Modify(index, DataHeader::Clear(update, change.update))
                    .serialize(self.id)
            })?;
            self.sender.send(message);
        }

        Ok(())
    }

    /// Replaces the header of `index` alone; queued at once like a clear.
    pub(crate) fn set_header(
        &self,
        index: u32,
        header: &[u8],
        update: bool,
    ) -> Result<(), CoreError> {
        self.header_type.check(&self.name, Some(header))?;
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        let value = w.values.get_mut(&index).ok_or_else(|| missing_key(index))?;
        value.store_header(Some(header));

        if w.sync.sendable(&self.connected).is_some()
            && let Some(bytes) = self.header_type.bytes(header)
        {
            let change = WireHeader::set(bytes.type_id, header);
            let message = change.message(|| {
                MultiDataHeader::Modify(index, DataHeader::Header(update, bytes)).serialize(self.id)
            })?;
            self.sender.send(message);
        }

        Ok(())
    }

    pub(crate) fn get<R>(&self, key: u32, f: impl FnOnce(Option<&[u8]>) -> R) -> R {
        let storage = self.storage.read();
        f(storage.values.get(&key).map(|value| value.bytes.as_slice()))
    }

    /// Calls `f` with the encoded header of `key`, `None` for a missing key;
    /// a state without a header type has an empty one.
    pub(crate) fn header<R>(&self, key: u32, f: impl FnOnce(Option<&[u8]>) -> R) -> R {
        let storage = self.storage.read();
        f(storage
            .values
            .get(&key)
            .map(|value| value.header.as_slice()))
    }

    /// Calls `f` with the bytes and the encoded header of `key` under one
    /// read lock, `None` for a missing key.
    pub(crate) fn get_with_header<R>(
        &self,
        key: u32,
        f: impl FnOnce(Option<(&[u8], &[u8])>) -> R,
    ) -> R {
        let storage = self.storage.read();
        f(storage
            .values
            .get(&key)
            .map(|value| (value.bytes.as_slice(), value.header.as_slice())))
    }

    fn send_after_ack(&self, generation: u64, messages: Vec<(FastVec<32>, bool)>) {
        send_after_ack(
            &self.storage,
            |storage| &storage.sync,
            &self.event,
            &self.connected,
            &self.sender,
            generation,
            messages,
        );
    }
}

impl Acknowledge for DataMulti {
    fn acknowledge(&self) {
        let mut w = self.sync_counter.lock();
        if *w > 0 {
            *w = w.saturating_sub(1);
            if *w > 0 {
                return;
            }
        }

        self.event.set();
    }

    fn reset(&self) {
        self.storage.write().sync.reset();
        // A connection sync waits for one acknowledgement per key, so the
        // default single `acknowledge` would leave a producer blocked.
        *self.sync_counter.lock() = 0;
        self.event.set();
    }
}

impl SyncTrait for DataMulti {
    fn sync(&self) -> Result<(), ()> {
        // See `Data::sync` for why the producer mutex is taken.
        let _producer = self.producer.lock();
        let mut w = self.storage.write();
        w.sync.synchronized();

        // The client keeps its keys across a disconnect; drop the ones removed
        // meanwhile before sending the current keys.
        let message = MultiDataHeader::Reset(false).serialize(self.id)?;
        self.sender.send(message);
        if w.values.is_empty() {
            self.event.set();
        } else {
            self.event.clear();
            let mut counter = self.sync_counter.lock();
            *counter = w.values.len();
            for (index, value) in w.values.iter() {
                let messages = pack_data(
                    self.id,
                    &value.bytes,
                    TransportType::Set(value.count as u64),
                    value.count as u64,
                    self.data_type,
                    Some(*index),
                    false,
                    self.header_type.change(Some(&value.header)),
                )
                .map_err(|_| ())?;

                for (message, single) in messages {
                    self.sender.send_set(message, single);
                }

                if !self.connected.load(Ordering::Acquire) {
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

// functions ------------------------------------------------
/// Waits until the client acknowledged the previous transfer, then queues
/// `messages` unless the connection ended or was replaced meanwhile.
///
/// The caller holds its state's producer mutex and read `generation` under
/// the storage lock together with its synchronization check. The storage read
/// guard is kept through enqueueing, so a reset cannot change the generation
/// between the check and the send.
fn send_after_ack<S>(
    storage: &RwLock<S>,
    sync: impl FnOnce(&S) -> &SyncState,
    event: &Event,
    connected: &AtomicBool,
    sender: &MessageSender,
    generation: u64,
    messages: Vec<(FastVec<32>, bool)>,
) {
    event.wait_clear();
    let storage = storage.read();
    if sync(&storage).generation() != generation || !connected.load(Ordering::Acquire) {
        return;
    }
    for (message, single) in messages {
        sender.send_set(message, single);
    }
}

/// Checks `data` against a state's element type and returns its element
/// count.
pub(crate) fn check_data_type(
    data: &DataSlice,
    data_type: DataType,
    item_size: usize,
) -> Result<usize, String> {
    if data.data_type != data_type {
        return Err(format!(
            "Data type mismatch: expected {:?}, got {:?}",
            data_type, data.data_type
        ));
    }

    if !data.bytes.len().is_multiple_of(item_size) {
        return Err(format!(
            "Data size must be a multiple of element size: expected multiple of {}, got {}",
            item_size,
            data.bytes.len()
        ));
    }

    Ok(data.bytes.len() / item_size)
}

/// Views native-endian `u16` values as a Data operation without a header.
#[cfg(test)]
pub(crate) fn u16_slice(values: &[u16]) -> DataSlice<'_> {
    // SAFETY: `u16` has no padding, so its values are plain bytes.
    let bytes =
        unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values)) };
    DataSlice {
        bytes,
        data_type: DataType::U16,
        header: None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pack_data(
    id: u64,
    data: &[u8],
    transport_type: TransportType,
    data_count: u64,
    data_type: DataType,
    multi_data: Option<u32>,
    update: bool,
    header: WireHeader,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_data_chunked(
        id,
        data,
        transport_type,
        data_count,
        data_type,
        multi_data,
        update,
        header,
        TRANSFER_CHUNK_SIZE,
    )
}

/// Packs a transfer; the header goes with the message that completes it, in
/// front of its elements.
#[allow(clippy::too_many_arguments)]
pub(in crate::server_core) fn pack_data_chunked(
    id: u64,
    data: &[u8],
    transport_type: TransportType,
    data_count: u64,
    data_type: DataType,
    multi_data: Option<u32>,
    update: bool,
    header: WireHeader,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    chunk_transfer(
        data.len(),
        data_type.item_size(),
        chunk_limit,
        |chunk, range| {
            let len = range.len() as u32;
            let (message, header_bytes) = match chunk {
                Chunk::All => (
                    DataHeader::All(data_type, transport_type, update, len, header.update),
                    header.bytes,
                ),
                Chunk::Start => (DataHeader::StartBatch(data_count, len), &[][..]),
                Chunk::Batch => (DataHeader::Batch(len), &[][..]),
                Chunk::End => (
                    DataHeader::End(data_type, transport_type, update, len, header.update),
                    header.bytes,
                ),
            };
            let mut message = match multi_data {
                None => message.serialize(id, true),
                Some(index) => MultiDataHeader::serialize_modify(id, index, message),
            }
            .map_err(|_| "Failed to serialize header".to_string())?;
            message.reserve_exact(header_bytes.len() + range.len());
            message.extend_from_slice(header_bytes);
            message.extend_from_slice(&data[range]);
            Ok(message)
        },
    )
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::time::Duration;

    use super::*;

    fn holder(values: &[u16]) -> DataSlice<'_> {
        u16_slice(values)
    }

    fn contents(message: &FastVec<32>) -> Vec<u8> {
        match message {
            FastVec::Heap(data) => data.clone(),
            FastVec::Stack(data) => data.as_ref().to_vec(),
        }
    }

    /// Every message queued so far, as plain bytes.
    fn queued(receiver: &mut crate::server_core::net::sender::MessageReceiver) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        while let Ok(Some((message, _))) = receiver.try_recv() {
            messages.push(contents(&message));
        }
        messages
    }

    fn packed(messages: Vec<(FastVec<32>, bool)>) -> Vec<Vec<u8>> {
        messages
            .iter()
            .map(|(message, _)| contents(message))
            .collect()
    }

    fn u16_bytes(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_ne_bytes()).collect()
    }

    /// Runs `f` on another thread and fails if it does not finish in time.
    fn within<T: Send + 'static>(label: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(f()).unwrap());
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| panic!("blocked: {label}"))
    }

    /// The connect window: the run loop marks the connection before it
    /// synchronizes. Producers in it used to wait for an acknowledgement while
    /// holding the storage read guard, a second producer queued for the write
    /// lock, and the synchronization's read lock queued behind it (D1). Now
    /// they only store, and the synchronization sends everything once (D8).
    #[test]
    fn producers_in_the_connect_window_only_store() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = Data::new(
            "root.data".into(),
            5,
            DataType::U16,
            None,
            sender,
            connected.clone(),
        );
        data.set(holder(&[1, 2]), false).unwrap();

        connected.store(true, Ordering::Release);
        let producers: Vec<_> = [vec![3, 4], vec![5, 6]]
            .into_iter()
            .map(|values| {
                let data = data.clone();
                std::thread::spawn(move || data.add(holder(&values), false))
            })
            .collect();
        std::thread::sleep(Duration::from_millis(100));
        let syncing = data.clone();
        within("Data sync behind window producers", move || syncing.sync()).unwrap();
        for producer in producers {
            within("Data window producer", move || producer.join().unwrap()).unwrap();
        }

        let stored = data.get(|bytes| bytes.unwrap().to_vec());
        assert_eq!(stored.len(), 12);
        let expected = pack_data(
            5,
            &stored,
            TransportType::Set(6),
            6,
            DataType::U16,
            None,
            false,
            WireHeader::KEEP,
        );
        assert_eq!(queued(&mut receiver), packed(expected.unwrap()));
    }

    #[test]
    fn multi_producers_in_the_connect_window_only_store() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = DataMulti::new(
            "root.data".into(),
            5,
            DataType::U16,
            None,
            sender,
            connected.clone(),
        );
        data.set(1, holder(&[1, 2]), false).unwrap();

        connected.store(true, Ordering::Release);
        let adding = data.clone();
        let add = std::thread::spawn(move || adding.add(1, holder(&[3]), false));
        let setting = data.clone();
        let set = std::thread::spawn(move || setting.set(1, holder(&[4, 5, 6]), false));
        std::thread::sleep(Duration::from_millis(100));
        let syncing = data.clone();
        within("DataMulti sync behind window producers", move || {
            syncing.sync()
        })
        .unwrap();
        within("DataMulti window add", move || add.join().unwrap()).unwrap();
        within("DataMulti window set", move || set.join().unwrap()).unwrap();

        let stored = data.get(1, |bytes| bytes.unwrap().to_vec());
        let count = stored.len() as u64 / 2;
        let expected = pack_data(
            5,
            &stored,
            TransportType::Set(count),
            count,
            DataType::U16,
            Some(1),
            false,
            WireHeader::KEEP,
        );
        let reset = MultiDataHeader::Reset(false).serialize(5).unwrap();
        let mut expected = packed(expected.unwrap());
        expected.insert(0, contents(&reset));
        assert_eq!(queued(&mut receiver), expected);
    }

    /// On a reconnect the reset leaves the event set, so a producer in the
    /// window used to send its delta at once, and the synchronization then
    /// sent the same change again inside its snapshot (D8).
    #[test]
    fn a_change_in_the_reconnect_window_is_sent_once() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = Data::new(
            "root.data".into(),
            5,
            DataType::U16,
            None,
            sender,
            connected.clone(),
        );
        data.set(holder(&[1, 2]), false).unwrap();
        connected.store(true, Ordering::Release);
        data.sync().unwrap();
        data.acknowledge();

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*data);
        queued(&mut receiver);
        connected.store(true, Ordering::Release);
        data.add(holder(&[7]), false).unwrap();
        data.remove(0, 1, None, false).unwrap();
        assert!(queued(&mut receiver).is_empty(), "a window change was sent");

        data.sync().unwrap();
        let expected = pack_data(
            5,
            &u16_bytes(&[2, 7]),
            TransportType::Set(2),
            2,
            DataType::U16,
            None,
            false,
            WireHeader::KEEP,
        );
        assert_eq!(queued(&mut receiver), packed(expected.unwrap()));

        // After the synchronization's acknowledgement, deltas flow again.
        data.acknowledge();
        data.add(holder(&[8]), false).unwrap();
        let expected = pack_data(
            5,
            &u16_bytes(&[8]),
            TransportType::Add(1),
            1,
            DataType::U16,
            None,
            false,
            WireHeader::KEEP,
        );
        assert_eq!(queued(&mut receiver), packed(expected.unwrap()));
    }

    /// An empty buffer that is set synchronizes as an empty set, which
    /// creates the data on the client; a clear never does.
    #[test]
    fn an_empty_buffer_synchronizes_as_a_set() {
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = Data::new(
            "root.data".into(),
            5,
            DataType::U16,
            None,
            sender,
            connected.clone(),
        );
        // Not connected yet: the set only stores.
        data.set(holder(&[]), false).unwrap();
        connected.store(true, Ordering::Release);
        data.sync().unwrap();

        let expected = pack_data(
            5,
            &[],
            TransportType::Set(0),
            0,
            DataType::U16,
            None,
            false,
            WireHeader::KEEP,
        );
        assert_eq!(queued(&mut receiver), packed(expected.unwrap()));
    }

    /// A producer that waited for an acknowledgement of a lost connection must
    /// not send its delta into the next one.
    #[test]
    fn a_producer_released_by_a_reset_drops_its_delta() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = Data::new(
            "root.data".into(),
            5,
            DataType::U16,
            None,
            sender,
            connected.clone(),
        );
        connected.store(true, Ordering::Release);
        data.sync().unwrap();
        // The synchronization of data that is not set needs no
        // acknowledgement; this set does.
        data.set(holder(&[1]), false).unwrap();
        queued(&mut receiver);

        let waiting = data.clone();
        let producer = std::thread::spawn(move || waiting.add(holder(&[2]), false));
        std::thread::sleep(Duration::from_millis(100));
        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*data);
        connected.store(true, Ordering::Release);
        within("producer released by reset", move || {
            producer.join().unwrap()
        })
        .unwrap();
        assert!(queued(&mut receiver).is_empty(), "a stale delta was sent");
        data.get(|bytes| assert_eq!(bytes, Some(&u16_bytes(&[1, 2])[..])));
    }

    /// The client keeps its keys across a disconnect, so a key removed while
    /// disconnected must be dropped by the synchronization (D9).
    #[test]
    fn multi_sync_resets_the_client_collection_before_the_keys() {
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = DataMulti::new(
            "root.data".into(),
            5,
            DataType::U16,
            None,
            sender,
            connected.clone(),
        );
        data.set(1, holder(&[1]), false).unwrap();
        data.set(2, holder(&[2]), false).unwrap();
        data.remove_index(2, false).unwrap();

        connected.store(true, Ordering::Release);
        data.sync().unwrap();
        let reset = MultiDataHeader::Reset(false).serialize(5).unwrap();
        let key = pack_data(
            5,
            &u16_bytes(&[1]),
            TransportType::Set(1),
            1,
            DataType::U16,
            Some(1),
            false,
            WireHeader::KEEP,
        );
        let mut expected = packed(key.unwrap());
        expected.insert(0, contents(&reset));
        assert_eq!(queued(&mut receiver), expected);
    }

    #[test]
    fn reset_releases_a_producer_waiting_for_a_multi_key_sync() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, _receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = DataMulti::new(
            "root.data".into(),
            5,
            DataType::U16,
            None,
            sender,
            connected.clone(),
        );
        data.set(1, holder(&[1, 2]), false).unwrap();
        data.set(2, holder(&[3, 4]), false).unwrap();
        data.set(3, holder(&[7, 8]), false).unwrap();

        connected.store(true, Ordering::Release);
        data.sync().unwrap();
        // Only one of the three sync acknowledgements arrives before the disconnect.
        data.acknowledge();

        let waiting = data.clone();
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            done_sender
                .send(waiting.set(1, holder(&[5, 6]), false))
                .unwrap();
        });
        assert!(
            done_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "the producer must wait for the remaining sync acknowledgements"
        );

        connected.store(false, Ordering::Release);
        Acknowledge::reset(&*data);
        done_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("reset did not release the waiting producer")
            .unwrap();
        waiter.join().unwrap();
        data.get(1, |v| assert_eq!(v, Some(&[5, 0, 6, 0][..])));
    }
}

#[cfg(all(test, feature = "client"))]
mod transfer_tests {
    use super::*;
    use crate::client::messages::{MessageSender as ClientSender, MessagesParser};
    use crate::client::states::data::{
        Data as ClientData, DataMulti as ClientMulti, UpdateData, UpdateMultiData,
    };
    use crate::protocol::{ClientHeader, ServerHeader};
    use crate::server_core::net::session::{Handshake, Session, run};
    use crate::server_core::net::transport::memory::{self, MemoryClient, MemoryConnector};
    use crate::server_core::net::transport::{Incoming, Outgoing};
    use crate::server_core::server::ServerStatesList;
    use crate::server_core::signals::SignalsManager;

    #[test]
    fn real_numeric_chunks_publish_complete_single_and_sparse_data() {
        for count in [
            TRANSFER_CHUNK_SIZE / 2 - 1,
            TRANSFER_CHUNK_SIZE / 2,
            TRANSFER_CHUNK_SIZE / 2 + 1,
            TRANSFER_CHUNK_SIZE + 17,
        ] {
            let expected: Vec<u16> = (0..count)
                .map(|i| (i.wrapping_mul(37) % 65521) as u16)
                .collect();
            for key in [None, Some(79)] {
                let (sender, mut receiver) = ClientSender::new();
                let single = ClientData::<u16>::new("single".into(), 51, sender.clone());
                let multi = ClientMulti::<u16>::new("multi".into(), 51, sender);
                // A second transfer demonstrates ACK and reconstruction state are reusable.
                for pass in 0..2 {
                    let payload: Vec<u16> =
                        expected.iter().map(|v| v.wrapping_add(pass * 3)).collect();
                    let bytes: Vec<u8> = payload.iter().flat_map(|v| v.to_ne_bytes()).collect();
                    let messages = pack_data(
                        51,
                        &bytes,
                        TransportType::Set(count as u64),
                        count as u64,
                        DataType::U16,
                        key,
                        true,
                        WireHeader::KEEP,
                    )
                    .unwrap();
                    let total = messages.len();
                    assert_eq!(total, bytes.len().div_ceil(TRANSFER_CHUNK_SIZE));
                    for (n, (message, _)) in messages.into_iter().enumerate() {
                        let (mut parser, (header, payload)) =
                            MessagesParser::from_bytes(message.into_bytes()).unwrap();
                        assert_eq!(header.update_flag(), n + 1 == total);
                        match header {
                            ServerHeader::Data(51, header) => {
                                single.update_data((header, payload)).unwrap();
                            }
                            ServerHeader::MultiData(51, MultiDataHeader::Modify(79, header)) => {
                                multi.update(79, (header, payload)).unwrap();
                            }
                            _ => panic!("wrong packed message for key {key:?}, chunk {n}"),
                        }
                        assert!(parser.next().unwrap().is_none());
                        if n + 1 < total {
                            assert!(receiver.try_recv().is_err(), "ACK before final chunk");
                            if key.is_none() {
                                single.read(|v| {
                                    assert_eq!(
                                        v,
                                        if pass == 0 {
                                            None
                                        } else {
                                            Some(expected.as_slice())
                                        }
                                    )
                                });
                            } else {
                                multi.read(79, |v| {
                                    assert_eq!(
                                        v,
                                        if pass == 0 {
                                            None
                                        } else {
                                            Some(expected.as_slice())
                                        }
                                    )
                                });
                            }
                        }
                    }
                    crate::test_support::ack(&mut receiver, 51);
                    if key.is_none() {
                        single.read(|v| assert_eq!(v, Some(payload.as_slice())));
                    } else {
                        multi.read(79, |v| assert_eq!(v, Some(payload.as_slice())));
                    }
                }
            }
        }
    }
    /// A deterministic xorshift generator.
    struct Random(u64);

    impl Random {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }

        fn values(&mut self) -> Vec<u16> {
            (0..self.below(5))
                .map(|_| self.below(1000) as u16)
                .collect()
        }
    }

    fn holder(values: &[u16]) -> DataSlice<'_> {
        u16_slice(values)
    }

    /// One random operation on the shared Data and DataMulti; failures such
    /// as an out-of-range index are part of the model and ignored.
    fn random_operation(random: &mut Random, data: &Data, multi: &DataMulti) {
        let values = random.values();
        let key = random.below(4) as u32;
        let len = data.get(|bytes| bytes.map_or(0, |b| b.len() / 2));
        let key_len = multi.get(key, |bytes| bytes.map_or(0, |b| b.len() / 2));
        let start = random.below(len + 1);
        let key_start = random.below(key_len + 1);
        let _ = match random.below(14) {
            // Sets come twice as often, so the data is mostly set.
            0 | 1 => data.set(holder(&values), false),
            2 => data.add(holder(&values), false),
            3 => data.replace(holder(&values), start, false),
            4 => data.remove(start, values.len().max(1), None, false),
            5 => data.clear(None, false),
            6 => data.reset(false),
            7 => multi.set(key, holder(&values), false),
            8 => multi.add(key, holder(&values), false),
            9 => multi.replace(key, key_start, holder(&values), false),
            10 => multi.remove(key, key_start, values.len().max(1), None, false),
            11 => multi.clear(key, None, false),
            12 => multi.remove_index(key, false).map_err(CoreError::from),
            _ => multi.reset(false).map_err(CoreError::from),
        };
    }

    /// The model test's client, connected to the server's real run loop
    /// through in-memory connections.
    struct Harness {
        connected: Arc<AtomicBool>,
        connector: MemoryConnector,
        client: Option<MemoryClient>,
        single: ClientData<u16>,
        keyed: ClientMulti<u16>,
        acks: tokio::sync::mpsc::UnboundedReceiver<Option<crate::client::messages::ChannelMessage>>,
    }

    impl Harness {
        /// Connects and waits until the run loop has accepted the client.
        fn connect(&mut self) {
            let client = self.connector.connect().unwrap();
            let handshake = ClientHeader::Handshake(crate::PROTOCOL_VERSION, None, None);
            let handshake = postcard::to_stdvec(&handshake).unwrap();
            client
                .send
                .send(Incoming::Binary(handshake.into()))
                .unwrap();
            self.client = Some(client);
            self.wait_until(true);
        }

        /// Drops the connection as a lost socket would: acknowledgements in
        /// flight are lost, and the client drops its partial transfers.
        fn disconnect(&mut self) {
            self.client = None;
            self.wait_until(false);
            self.single.disconnected();
            self.keyed.disconnected();
            while let Ok(Some(_)) = self.acks.try_recv() {}
        }

        fn wait_until(&self, connected: bool) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while self.connected.load(Ordering::Acquire) != connected {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the run loop did not reach connected = {connected}"
                );
                std::thread::yield_now();
            }
        }

        /// Delivers one frame to the client and the client's
        /// acknowledgements to the server; returns whether anything moved.
        fn pump(&mut self) -> bool {
            use crate::client::messages::ChannelMessage;

            let client = self.client.as_mut().unwrap();
            let mut moved = false;
            if let Ok(frame) = client.frames.try_recv() {
                moved = true;
                let Outgoing::Binary(frame) = frame else {
                    panic!("unexpected ping without a heartbeat");
                };
                let (mut parser, first) = MessagesParser::from_bytes(frame).unwrap();
                let mut next = Some(first);
                while let Some((header, payload)) = next {
                    match header {
                        ServerHeader::Update(_) => {}
                        ServerHeader::Data(51, header) => {
                            self.single.update_data((header, payload)).unwrap()
                        }
                        ServerHeader::MultiData(52, MultiDataHeader::Modify(key, header)) => {
                            self.keyed.update(key, (header, payload)).unwrap()
                        }
                        ServerHeader::MultiData(52, MultiDataHeader::Remove(key, _)) => {
                            self.keyed.remove(key)
                        }
                        ServerHeader::MultiData(52, MultiDataHeader::Reset(_)) => {
                            self.keyed.reset()
                        }
                        _ => panic!("unexpected server message"),
                    }
                    next = parser.next().unwrap();
                }
            }
            while let Ok(Some(message)) = self.acks.try_recv() {
                if let ChannelMessage::Ack(id) = message {
                    moved = true;
                    let ack = postcard::to_stdvec(&ClientHeader::Ack(id)).unwrap();
                    // The connection may already be closing.
                    let _ = client.send.send(Incoming::Binary(ack.into()));
                }
            }
            moved
        }
    }

    /// Producers race connects, acknowledgements and disconnects through the
    /// server's run loop. At rest the real client handles must hold exactly
    /// the server's data: a change sent both as a delta and inside a
    /// synchronization (D8), or a key the synchronization left on the client
    /// (D9), shows up as a difference.
    #[test]
    fn producers_and_reconnects_keep_the_client_equal_to_the_server() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, queue) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = Data::new(
            "data".into(),
            51,
            DataType::U16,
            None,
            sender.clone(),
            connected.clone(),
        );
        let multi = DataMulti::new(
            "multi".into(),
            52,
            DataType::U16,
            None,
            sender.clone(),
            connected.clone(),
        );
        let mut values = ServerStatesList {
            sync: vec![data.clone(), multi.clone()],
            ..Default::default()
        };
        values.ack.insert(51, data.clone());
        values.ack.insert(52, multi.clone());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (listener, connector) = memory::listener();
        let stop_event = Event::new();
        let run_loop = runtime.spawn(run(
            listener,
            queue,
            stop_event.clone(),
            Handshake {
                version: None,
                token: None,
                origins: Default::default(),
            },
            Default::default(),
            Session {
                sender: sender.clone(),
                connected: connected.clone(),
                values,
                signals: SignalsManager::new(),
                heartbeat: None,
                transfers: crate::file_transfer::engine::Engine::new(
                    crate::file_transfer::engine::Side::Server,
                    None,
                ),
            },
        ));

        let (client_sender, acks) = ClientSender::new();
        let mut harness = Harness {
            connected: connected.clone(),
            connector,
            client: None,
            single: ClientData::new("data".into(), 51, client_sender.clone()),
            keyed: ClientMulti::new("multi".into(), 52, client_sender),
            acks,
        };

        let stop = Arc::new(AtomicBool::new(false));
        let producers: Vec<_> = (0..3_u64)
            .map(|seed| {
                let (data, multi) = (data.clone(), multi.clone());
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut random = Random(0x9e37_79b9_7f4a_7c15 ^ (seed + 1));
                    while !stop.load(Ordering::Acquire) {
                        random_operation(&mut random, &data, &multi);
                        if random.below(8) == 0 {
                            std::thread::yield_now();
                        }
                    }
                })
            })
            .collect();

        let mut random = Random(0x2545_f491_4f6c_dd1d);
        const CONNECTIONS: usize = 200;
        for _ in 0..CONNECTIONS {
            harness.connect();
            for _ in 0..random.below(200) {
                if !harness.pump() {
                    std::thread::yield_now();
                }
            }
            harness.disconnect();
            // Let producers change the states while no client is connected.
            std::thread::sleep(std::time::Duration::from_micros(random.below(300) as u64));
        }
        // The last disconnect released every producer waiting for an
        // acknowledgement, so they all see the stop flag.
        stop.store(true, Ordering::Release);
        for producer in producers {
            producer.join().unwrap();
        }

        // A final connection, run until nothing moves any more.
        harness.connect();
        let mut idle = 0;
        while idle < 50 {
            if harness.pump() {
                idle = 0;
            } else {
                idle += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }

        let server = data.get(|bytes| bytes.map(<[u8]>::to_vec));
        let client = harness.single.get().map(|values| {
            values
                .iter()
                .flat_map(|v| v.to_ne_bytes())
                .collect::<Vec<u8>>()
        });
        assert_eq!(client, server, "Data diverged");
        for key in 0..4 {
            let server = multi.get(key, |bytes| bytes.map(<[u8]>::to_vec));
            let client = harness.keyed.get(key).map(|values| {
                values
                    .iter()
                    .flat_map(|v| v.to_ne_bytes())
                    .collect::<Vec<u8>>()
            });
            assert_eq!(client, server, "DataMulti key {key} diverged");
        }

        connected.store(false, Ordering::Release);
        stop_event.set();
        sender.close();
        runtime.block_on(run_loop).unwrap();
    }

    /// Chunked `Add` and `Replace` transfers must extend and patch the existing
    /// data only after their final chunk, like a chunked `Set`.
    #[test]
    fn real_numeric_chunks_apply_add_and_replace() {
        let count = TRANSFER_CHUNK_SIZE / 2 + 1;
        let large: Vec<u16> = (0..count).map(|i| (i % 65521) as u16).collect();
        let patch: Vec<u16> = large.iter().map(|v| v.wrapping_mul(3)).collect();
        for key in [None, Some(79)] {
            let (sender, mut receiver) = ClientSender::new();
            let single = ClientData::<u16>::new("single".into(), 52, sender.clone());
            let multi = ClientMulti::<u16>::new("multi".into(), 52, sender);
            let mut expected = Vec::new();
            let read = |expected: &[u16]| {
                if key.is_none() {
                    single.read(|v| assert!(v == Some(expected), "single data diverged"));
                } else {
                    multi.read(79, |v| assert!(v == Some(expected), "keyed data diverged"));
                }
            };
            let operations = [
                (TransportType::Set(3), vec![7, 8, 9]),
                (TransportType::Add(count as u64), large.clone()),
                (TransportType::Replace(2, count as u64), patch.clone()),
            ];
            for (transport, values) in operations {
                let before = expected.clone();
                match transport {
                    TransportType::Set(_) => expected = values.clone(),
                    TransportType::Add(_) => expected.extend(&values),
                    TransportType::Replace(start, _) => {
                        let start = start as usize;
                        expected[start..start + values.len()].copy_from_slice(&values);
                    }
                }
                let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
                let messages = pack_data(
                    52,
                    &bytes,
                    transport,
                    values.len() as u64,
                    DataType::U16,
                    key,
                    false,
                    WireHeader::KEEP,
                )
                .unwrap();
                let total = messages.len();
                for (n, (message, _)) in messages.into_iter().enumerate() {
                    let (_, (header, payload)) =
                        MessagesParser::from_bytes(message.into_bytes()).unwrap();
                    match header {
                        ServerHeader::Data(52, header) => {
                            single.update_data((header, payload)).unwrap()
                        }
                        ServerHeader::MultiData(52, MultiDataHeader::Modify(79, header)) => {
                            multi.update(79, (header, payload)).unwrap()
                        }
                        _ => panic!("wrong packed message for key {key:?}"),
                    }
                    if n + 1 < total {
                        assert!(receiver.try_recv().is_err(), "ACK before final chunk");
                        read(&before);
                    }
                }
                crate::test_support::ack(&mut receiver, 52);
                read(&expected);
            }
        }
    }
}

/// Headers from the server cores to the real client handles.
#[cfg(all(test, feature = "client"))]
mod header_tests {
    use super::*;
    use crate::client::messages::{ChannelMessage, MessageSender as ClientSender, MessagesParser};
    use crate::client::states::data::{
        Data as ClientData, DataMulti as ClientMulti, UpdateData, UpdateMultiData,
    };
    use crate::client::states::data_take::{DataTake as ClientTake, UpdateDataTake};
    use crate::protocol::{ServerHeader, VALUE_MAX_SIZE};
    use crate::server_core::error::ErrorKind;
    use crate::server_core::net::sender::MessageReceiver;
    use crate::server_core::states::data_take::{DataMultiTake, DataTake};
    use crate::typed::Typed;

    type Meta = (u32, String);

    fn meta(n: u32) -> Meta {
        (n, format!("m{n}"))
    }

    fn encoded(n: u32) -> Vec<u8> {
        postcard::to_stdvec(&meta(n)).unwrap()
    }

    fn type_id() -> Option<u32> {
        Some(<Meta as Typed>::get_type().get_hash())
    }

    fn with<'a>(values: &'a [u16], header: Option<&'a [u8]>) -> DataSlice<'a> {
        DataSlice {
            header,
            ..u16_slice(values)
        }
    }

    /// Feeds every queued message to the client handles and returns how many
    /// acknowledgements they sent.
    fn deliver(
        receiver: &mut MessageReceiver,
        acks: &mut tokio::sync::mpsc::UnboundedReceiver<Option<ChannelMessage>>,
        single: Option<&ClientData<u16, Meta>>,
        multi: Option<&ClientMulti<u16, Meta>>,
    ) -> usize {
        while let Ok(Some((message, _))) = receiver.try_recv() {
            let (mut parser, first) = MessagesParser::from_bytes(message.into_bytes()).unwrap();
            let mut next = Some(first);
            while let Some((header, payload)) = next {
                match header {
                    ServerHeader::Data(_, header) => {
                        single.unwrap().update_data((header, payload)).unwrap()
                    }
                    ServerHeader::MultiData(_, MultiDataHeader::Modify(key, header)) => {
                        multi.unwrap().update(key, (header, payload)).unwrap()
                    }
                    ServerHeader::MultiData(_, MultiDataHeader::Remove(key, _)) => {
                        multi.unwrap().remove(key)
                    }
                    ServerHeader::MultiData(_, MultiDataHeader::Reset(_)) => multi.unwrap().reset(),
                    _ => panic!("unexpected server message"),
                }
                next = parser.next().unwrap();
            }
        }
        let mut count = 0;
        while let Ok(Some(ChannelMessage::Ack(_))) = acks.try_recv() {
            count += 1;
        }
        count
    }

    #[test]
    fn a_data_header_is_published_with_its_buffer() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(true));
        let data = Data::new(
            "root.data".into(),
            5,
            DataType::U16,
            type_id(),
            sender,
            connected,
        );
        let (client_sender, mut acks) = ClientSender::new();
        let client = ClientData::<u16, Meta>::new("root.data".into(), 5, client_sender);
        let mut deliver =
            |single: &ClientData<u16, Meta>| deliver(&mut receiver, &mut acks, Some(single), None);

        // A synchronization of data that is not set says so.
        data.sync().unwrap();
        deliver(&client);
        assert_eq!(client.get_with_header(), None);
        data.header(|header| assert_eq!(header, None));

        data.set(with(&[1, 2], Some(&encoded(1))), false).unwrap();
        assert_eq!(deliver(&client), 1);
        client.read_with_header(|entry| {
            assert_eq!(entry, Some((&[1, 2][..], &meta(1))));
        });
        data.acknowledge();

        // Without a header, a change keeps the current one.
        data.add(with(&[3], None), false).unwrap();
        deliver(&client);
        data.acknowledge();
        assert_eq!(client.get_with_header(), Some((vec![1, 2, 3], meta(1))));
        data.get_with_header(|entry| {
            assert_eq!(entry, Some((u16_slice(&[1, 2, 3]).bytes, &encoded(1)[..])));
        });

        data.remove(0, 1, Some(&encoded(2)), false).unwrap();
        deliver(&client);
        data.acknowledge();
        assert_eq!(
            (client.get(), client.header()),
            (Some(vec![2, 3]), Some(meta(2)))
        );

        // A header alone needs no acknowledgement.
        data.set_header(&encoded(3), false).unwrap();
        assert_eq!(deliver(&client), 0);
        assert_eq!(client.get_with_header(), Some((vec![2, 3], meta(3))));

        // A clear keeps the data set, with an empty buffer.
        data.clear(None, false).unwrap();
        deliver(&client);
        assert_eq!(client.get_with_header(), Some((vec![], meta(3))));
        data.clear(Some(&encoded(4)), false).unwrap();
        deliver(&client);
        assert_eq!(client.header(), Some(meta(4)));
        data.header(|header| assert_eq!(header, Some(&encoded(4)[..])));

        // A reconnecting client gets the stored header, with the empty
        // buffer: a set, which creates its data and which it acknowledges.
        Acknowledge::reset(&*data);
        let (client_sender, mut fresh_acks) = ClientSender::new();
        let fresh = ClientData::<u16, Meta>::new("root.data".into(), 5, client_sender);
        data.sync().unwrap();
        deliver(&fresh);
        assert_eq!(fresh.get_with_header(), Some((vec![], meta(4))));
        assert!(matches!(
            fresh_acks.try_recv(),
            Ok(Some(ChannelMessage::Ack(5)))
        ));
        data.acknowledge();

        data.set(with(&[9], None), false).unwrap();
        deliver(&fresh);
        data.acknowledge();
        Acknowledge::reset(&*data);
        let (client_sender, _acks) = ClientSender::new();
        let fresh = ClientData::<u16, Meta>::new("root.data".into(), 5, client_sender);
        data.sync().unwrap();
        deliver(&fresh);
        assert_eq!(fresh.get_with_header(), Some((vec![9], meta(4))));
    }

    #[test]
    fn a_reset_returns_the_data_to_not_set() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(true));
        let data = Data::new(
            "root.data".into(),
            5,
            DataType::U16,
            type_id(),
            sender,
            connected,
        );
        let (client_sender, mut acks) = ClientSender::new();
        let client = ClientData::<u16, Meta>::new("root.data".into(), 5, client_sender);
        data.sync().unwrap();
        deliver(&mut receiver, &mut acks, Some(&client), None);

        data.set(with(&[1], Some(&encoded(1))), false).unwrap();
        deliver(&mut receiver, &mut acks, Some(&client), None);
        data.acknowledge();

        // A reset needs no acknowledgement.
        data.reset(true).unwrap();
        assert_eq!(deliver(&mut receiver, &mut acks, Some(&client), None), 0);
        assert_eq!(client.get_with_header(), None);
        data.get_with_header(|entry| assert_eq!(entry, None));

        // Resetting data that is not set sends nothing.
        data.reset(false).unwrap();
        assert!(receiver.try_recv().is_err(), "a message was queued");

        // The set that creates the data again needs a header.
        let error = data.set(with(&[2], None), false).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidArgument);
        data.set(with(&[2], Some(&encoded(2))), false).unwrap();
        assert_eq!(deliver(&mut receiver, &mut acks, Some(&client), None), 1);
        data.acknowledge();
        assert_eq!(client.get_with_header(), Some((vec![2], meta(2))));

        // A client keeps its data across a disconnect; the synchronization
        // tells it that the data was reset meanwhile.
        Acknowledge::reset(&*data);
        data.reset(false).unwrap();
        assert!(receiver.try_recv().is_err(), "a message was queued");
        assert_eq!(client.get_with_header(), Some((vec![2], meta(2))));
        data.sync().unwrap();
        assert_eq!(deliver(&mut receiver, &mut acks, Some(&client), None), 0);
        assert_eq!(client.get_with_header(), None);
    }

    #[test]
    fn data_that_is_not_set_is_created_only_by_a_set_with_its_header() {
        let (sender, _receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let data = Data::new(
            "root.data".into(),
            6,
            DataType::U16,
            type_id(),
            sender.clone(),
            connected.clone(),
        );
        let header = encoded(1);
        let error = data.set(with(&[1], None), false).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidArgument);
        assert!(error.message.contains("needs a header"), "{error}");
        for error in [
            data.add(with(&[1], Some(&header)), false).unwrap_err(),
            data.replace(with(&[], None), 0, false).unwrap_err(),
            data.remove(0, 1, None, false).unwrap_err(),
            data.clear(Some(&header), false).unwrap_err(),
            data.set_header(&header, false).unwrap_err(),
        ] {
            assert_eq!(error.kind, ErrorKind::NotFound, "{error}");
            assert!(error.message.contains("is not set"), "{error}");
        }
        data.get_with_header(|entry| assert_eq!(entry, None));

        // Empty data is set, unlike no data.
        data.set(with(&[], Some(&header)), false).unwrap();
        data.get_with_header(|entry| assert_eq!(entry, Some((&[][..], &header[..]))));

        // Without a header type, a set creates the data alone.
        let plain = Data::new(
            "root.plain".into(),
            7,
            DataType::U16,
            None,
            sender,
            connected,
        );
        plain.set(with(&[3], None), false).unwrap();
        plain.get_with_header(|entry| {
            assert_eq!(entry, Some((u16_slice(&[3]).bytes, &[][..])));
        });
    }

    #[test]
    fn headers_are_checked_before_anything_is_stored() {
        let (sender, _receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let plain = Data::new(
            "root.plain".into(),
            5,
            DataType::U16,
            None,
            sender.clone(),
            connected.clone(),
        );
        let header = encoded(1);
        for error in [
            plain.set(with(&[1], Some(&header)), false).unwrap_err(),
            plain.clear(Some(&header), false).unwrap_err(),
            plain.set_header(&header, false).unwrap_err(),
        ] {
            assert!(
                error.message.contains("registered without a header"),
                "{error}"
            );
        }
        plain.get(|bytes| assert_eq!(bytes, None));

        let data = Data::new(
            "root.data".into(),
            6,
            DataType::U16,
            type_id(),
            sender,
            connected,
        );
        data.set(with(&[], Some(&header)), false).unwrap();
        let largest = vec![0; VALUE_MAX_SIZE];
        data.set_header(&largest, false).unwrap();
        let error = data
            .set_header(&[0; VALUE_MAX_SIZE + 1], false)
            .unwrap_err();
        assert!(error.message.contains("root.data header"), "{error}");
        assert!(error.message.contains("too large"), "{error}");
        data.header(|header| assert_eq!(header.map(<[u8]>::len), Some(VALUE_MAX_SIZE)));
    }

    #[test]
    fn every_key_has_its_own_header() {
        if crate::test_support::isolated() {
            return;
        }
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(true));
        let data = DataMulti::new(
            "root.multi".into(),
            5,
            DataType::U16,
            type_id(),
            sender,
            connected,
        );
        let (client_sender, mut acks) = ClientSender::new();
        let client = ClientMulti::<u16, Meta>::new("root.multi".into(), 5, client_sender);
        let mut deliver =
            |multi: &ClientMulti<u16, Meta>| deliver(&mut receiver, &mut acks, None, Some(multi));
        data.sync().unwrap();
        deliver(&client);

        // A new key needs a header.
        let error = data.set(1, with(&[1], None), false).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidArgument);
        assert!(error.message.contains("index 1"), "{error}");
        data.header(1, |header| assert_eq!(header, None));
        data.set(1, with(&[1], Some(&encoded(0))), false).unwrap();
        deliver(&client);
        data.acknowledge();
        assert_eq!(client.header(1), Some(meta(0)));
        data.header(1, |header| assert_eq!(header, Some(&encoded(0)[..])));

        data.set_header(1, &encoded(1), false).unwrap();
        deliver(&client);
        data.set(1, with(&[2, 3], None), false).unwrap();
        deliver(&client);
        data.acknowledge();
        client.read_with_header(1, |entry| {
            assert_eq!(entry, Some((&[2, 3][..], &meta(1))));
        });
        assert_eq!(client.get_with_header(1), Some((vec![2, 3], meta(1))));
        client.read_header(1, |header| assert_eq!(header, Some(&meta(1))));
        data.get_with_header(1, |entry| {
            assert_eq!(entry, Some((u16_slice(&[2, 3]).bytes, &encoded(1)[..])));
        });

        data.set(2, with(&[4], Some(&encoded(2))), false).unwrap();
        deliver(&client);
        data.acknowledge();
        let mut seen = Vec::new();
        client.for_each_with_header(|key, values, header| {
            seen.push((key, values.to_vec(), header.clone()))
        });
        seen.sort();
        assert_eq!(seen, [(1, vec![2, 3], meta(1)), (2, vec![4], meta(2))]);
        client.read_all_with_header(|data, headers| {
            assert_eq!((data.len(), data.get(&2)), (2, Some(&vec![4])));
            assert_eq!((headers.len(), headers.get(&2)), (2, Some(&meta(2))));
        });

        let error = data.set_header(9, &encoded(9), false).unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);

        // Removing a key drops its header on both sides.
        data.remove_index(1, false).unwrap();
        deliver(&client);
        assert_eq!(client.header(1), None);
        assert_eq!(client.get_with_header(1), None);
        client.read_header(1, |header| assert_eq!(header, None));
        data.header(1, |header| assert_eq!(header, None));
        data.get_with_header(1, |entry| assert_eq!(entry, None));

        // A reconnecting client gets every key with its stored header.
        data.set(3, with(&[5], Some(&encoded(5))), false).unwrap();
        deliver(&client);
        data.acknowledge();
        Acknowledge::reset(&*data);
        let (client_sender, _acks) = ClientSender::new();
        let fresh = ClientMulti::<u16, Meta>::new("root.multi".into(), 5, client_sender);
        data.sync().unwrap();
        deliver(&fresh);
        assert_eq!(fresh.header(2), Some(meta(2)));
        assert_eq!(fresh.header(3), Some(meta(5)));
        assert_eq!(fresh.header(1), None);
    }

    #[test]
    fn a_take_brings_its_header_also_from_the_cache() {
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(true));
        let take = DataTake::new(
            "root.take".into(),
            5,
            DataType::U16,
            type_id(),
            sender.clone(),
            connected.clone(),
        );
        let (client_sender, _acks) = ClientSender::new();
        let client = ClientTake::<u16, Meta>::new("root.take".into(), 5, client_sender);
        let mut deliver = || {
            while let Ok(Some((message, _))) = receiver.try_recv() {
                let (_, (header, payload)) =
                    MessagesParser::from_bytes(message.into_bytes()).unwrap();
                let ServerHeader::DataTake(_, header, blocking) = header else {
                    panic!("unexpected server message");
                };
                client.update((header, payload), blocking).unwrap();
            }
        };
        take.sync().unwrap();

        take.set(with(&[1, 2], Some(&encoded(1))), false, false, true)
            .unwrap();
        deliver();
        assert_eq!(client.take_with_header(), Some((vec![1, 2], meta(1))));

        // Every take needs a header.
        let error = take.set(with(&[3], None), false, false, true).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidArgument);
        deliver();
        assert_eq!(client.take_with_header(), None);

        // A reconnect replays the cache with its header.
        take.set(with(&[4], Some(&encoded(4))), false, false, true)
            .unwrap();
        deliver();
        client.take();
        Acknowledge::reset(&*take);
        take.sync().unwrap();
        deliver();
        assert_eq!(client.take_with_header(), Some((vec![4], meta(4))));

        let multi = DataMultiTake::new(
            "root.multi_take".into(),
            6,
            DataType::U16,
            type_id(),
            sender,
            connected,
        );
        let error = multi
            .set(1, with(&[1], None), false, false, true)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidArgument);
    }
}
