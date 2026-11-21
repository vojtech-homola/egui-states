//! Read-only client handles for server-controlled numeric buffers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use super::ChangedFlag;
use super::header::{
    HeaderValue, Label, created_header, decode_header, decode_header_bytes, header_type_id,
    split_header,
};
use crate::client::messages::{ChannelMessage, MessageSender};
use crate::hashing::NoHashMap;
use crate::protocol::data::{DataHeader, DataType, HeaderUpdate, TransportType};
use crate::typed::ObjectType;

/// A Data message: its wire header and the payload that followed it.
pub(crate) type DataMessage = (DataHeader, Bytes);

pub(crate) mod private {
    use super::DataType;

    /// # Safety
    ///
    /// The type must be plain old data valid for any bit pattern, and
    /// `get_type` must describe its exact size, because received bytes are
    /// copied straight into buffers of it.
    pub(crate) unsafe trait GetDataType: Clone + Copy {
        fn get_type() -> DataType;

        fn get_type_id() -> u32 {
            Self::get_type().id().into()
        }
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

/// Appends the elements encoded in `data`, which holds a whole number of them.
fn extend_from_bytes<T: private::GetDataType>(target: &mut Vec<T>, data: &[u8]) {
    debug_assert!(data.len().is_multiple_of(size_of::<T>()));
    let count = data.len() / size_of::<T>();
    target.reserve(count);
    // SAFETY: a `GetDataType` type is plain old data of `size_of::<T>()`
    // bytes and valid for any bit pattern; `reserve` made room for `count`
    // more elements, and a received payload cannot overlap the vector.
    unsafe {
        std::ptr::copy_nonoverlapping(
            data.as_ptr(),
            target.as_mut_ptr().add(target.len()).cast::<u8>(),
            data.len(),
        );
        target.set_len(target.len() + count);
    }
}

/// The number of whole elements in `data`; `what` names the payload in the
/// error.
fn element_count<T>(data: &[u8], what: &str) -> Result<usize, String> {
    let size = size_of::<T>();
    if !data.len().is_multiple_of(size) {
        return Err(format!(
            "{what} size {} is not a multiple of element size {size}",
            data.len()
        ));
    }
    Ok(data.len() / size)
}

/// Overwrites `target` with the elements encoded in `data`, which holds
/// exactly as many.
fn copy_from_bytes<T: private::GetDataType>(target: &mut [T], data: &[u8]) {
    assert_eq!(size_of_val(target), data.len());
    // SAFETY: as in `extend_from_bytes`; the lengths were checked.
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), target.as_mut_ptr().cast::<u8>(), data.len());
    }
}

/// The elements of a complete transfer: the bytes of a single message, which
/// hold a whole number of elements, or the buffer a batch assembled.
///
/// A single message is copied straight into the stored buffer for `Add` and
/// `Replace`, without an intermediate vector.
enum Payload<'a, T> {
    Bytes(&'a [u8]),
    Buffer(Vec<T>),
}

impl<T: private::GetDataType> Payload<'_, T> {
    fn len(&self) -> usize {
        match self {
            Self::Bytes(data) => data.len() / size_of::<T>(),
            Self::Buffer(buffer) => buffer.len(),
        }
    }

    fn into_vec(self) -> Vec<T> {
        match self {
            Self::Bytes(data) => {
                let mut buffer = Vec::new();
                extend_from_bytes(&mut buffer, data);
                buffer
            }
            Self::Buffer(buffer) => buffer,
        }
    }

    fn append_to(self, target: &mut Vec<T>) {
        match self {
            Self::Bytes(data) => extend_from_bytes(target, data),
            Self::Buffer(buffer) => target.extend_from_slice(&buffer),
        }
    }

    /// Overwrites `target`, which has the payload's length.
    fn write_to(self, target: &mut [T]) {
        match self {
            Self::Bytes(data) => copy_from_bytes(target, data),
            Self::Buffer(buffer) => target.copy_from_slice(&buffer),
        }
    }
}

/// Copies the payload of a single-message transfer into a new buffer.
pub(crate) fn elements<T: private::GetDataType>(data: &[u8]) -> Result<Vec<T>, String> {
    element_count::<T>(data, "Data")?;
    let mut buffer = Vec::new();
    extend_from_bytes(&mut buffer, data);
    Ok(buffer)
}

/// A numeric buffer being assembled from the chunks of one transfer.
///
/// The element count the batch start announced bounds every later chunk, so
/// no chunk is accepted past the transfer's size, whatever capacity the
/// allocator handed out.
pub(crate) struct BatchBuffer<T> {
    data: Vec<T>,
    total: usize,
}

impl<T: private::GetDataType> BatchBuffer<T> {
    /// Allocates a transfer of `count` elements and appends its first chunk.
    ///
    /// `count` comes from the network, so a size that overflows or cannot be
    /// allocated is an error rather than a panic.
    pub(crate) fn start(data: &[u8], count: u64) -> Result<Self, String> {
        let all_data_size = count
            .checked_mul(size_of::<T>() as u64)
            .ok_or_else(|| format!("Batch start count {count} overflows the data size"))?;
        if data.len() as u64 > all_data_size {
            return Err(format!(
                "Batch start data size {} exceeds total data size {}",
                data.len(),
                all_data_size
            ));
        }
        element_count::<T>(data, "Batch start data")?;

        let cannot_allocate = || format!("Cannot allocate a batch buffer for {count} elements");
        let total = usize::try_from(count).map_err(|_| cannot_allocate())?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(total)
            .map_err(|_| cannot_allocate())?;
        extend_from_bytes(&mut buffer, data);
        Ok(Self {
            data: buffer,
            total,
        })
    }

    /// Appends a chunk between the first and the last.
    pub(crate) fn append(&mut self, data: &[u8]) -> Result<(), String> {
        let len = self.data.len() + element_count::<T>(data, "Batch data")?;
        if len > self.total {
            return Err(format!(
                "Batch data size {} exceeds total data size {}",
                len, self.total
            ));
        }
        extend_from_bytes(&mut self.data, data);
        Ok(())
    }

    /// Appends the last chunk and returns the complete buffer.
    pub(crate) fn finish(mut self, data: &[u8]) -> Result<Vec<T>, String> {
        let len = self.data.len() + element_count::<T>(data, "Batch data")?;
        if len > self.total {
            return Err(format!(
                "Batch end data size {} exceeds total data size {}",
                len, self.total
            ));
        }
        if len != self.total {
            return Err(format!(
                "Batch end data size {} does not match total data size {}",
                len, self.total
            ));
        }
        extend_from_bytes(&mut self.data, data);
        Ok(self.data)
    }
}

/// Checks the element count a transport type announced against its payload.
fn check_count<T: private::GetDataType>(
    data: &Payload<'_, T>,
    count: u64,
    transport: &str,
) -> Result<(), String> {
    if data.len() as u64 != count {
        return Err(format!(
            "Data size {} does not match expected count {} for {transport} transport type",
            data.len(),
            count
        ));
    }
    Ok(())
}

/// Converts a range received from the network to indexes into a buffer of
/// `len` elements, or `None` if it overflows or ends past the buffer.
fn checked_range(start: u64, count: u64, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(start).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    (end <= len).then_some(start..end)
}

/// The layout hash type id of a Data state of elements `T` with header `H`.
/// A state without a header keeps the element type's id.
pub(crate) fn data_type_hash<T: private::GetDataType, H: HeaderValue>() -> u32 {
    match H::get_type() {
        ObjectType::Empty => T::get_type_id(),
        object_type => object_type.get_hash_from(T::get_type_id()),
    }
}

// Data -------------------------------------------------------------------
pub(crate) trait UpdateData: Sync + Send {
    fn update_data(&self, message: DataMessage) -> Result<(), String>;
    /// Drops a partial transfer, which can never complete once its
    /// connection is closed.
    fn disconnected(&self);
}

/// The buffer of a [`Data`] and its header, changed together.
struct Store<T, H> {
    data: Vec<T>,
    header: H,
}

/// A client-side, contiguous numeric buffer synchronized from the server,
/// with an optional typed header `H`.
///
/// Supported element types are `u8` through `u64`, `i8` through `i64`, `f32`,
/// and `f64`. The data is "not set", and every getter returns `None`, until
/// the server sets it, so an empty buffer the server sent differs from no
/// data. The server sets the header with the data and can return both to
/// "not set". After a disconnect the client keeps the last data until the
/// next connection says what it is.
///
/// The server may send complete or partial updates and split large
/// transfers into batches; the client publishes a completed batch as one
/// buffer. A header that arrives with a transfer is published together with
/// it, so readers never see the new header with the old buffer. The client
/// API is read-only.
///
/// Clone the handle freely; all clones refer to the same buffer.
pub struct Data<T, H = ()> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    header_type: Option<u32>,
    inner: Arc<RwLock<Option<Store<T, H>>>>,
    buffer: Arc<Mutex<Option<BatchBuffer<T>>>>,
    sender: MessageSender,
    changed: ChangedFlag,
}

#[allow(private_bounds)]
impl<T, H> Data<T, H>
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
            buffer: Arc::new(Mutex::new(None)),
            sender,
            changed: ChangedFlag::default(),
        }
    }

    /// Tells whether the server changed the data or its header since the
    /// last call, and clears the flag.
    ///
    /// Only this call clears it; reading the data does not. Call it once per
    /// frame, before reading, and use the result wherever the UI rebuilds
    /// something from the data, such as a plot: a second call in the same
    /// frame returns `false`. The synchronization after a connection also
    /// counts as a change, and so does a reset to "not set".
    ///
    /// ```no_run
    /// # fn downsample(samples: &[f32]) -> Vec<f32> { samples.to_vec() }
    /// # fn example(samples: egui_states::Data<f32>, plot: &mut Vec<f32>) {
    /// if samples.take_changed() {
    ///     *plot = samples.read(|samples| samples.map(downsample).unwrap_or_default());
    /// }
    /// # }
    /// ```
    pub fn take_changed(&self) -> bool {
        self.changed.take()
    }

    /// Returns a copy of the complete buffer, or `None` while the data is not
    /// set.
    ///
    /// This allocates and copies every element. Use [`Self::read`] to inspect a
    /// large buffer without cloning it, and [`Self::get_with_header`] to copy
    /// the header with the buffer.
    pub fn get(&self) -> Option<Vec<T>> {
        self.inner.read().as_ref().map(|store| store.data.clone())
    }

    /// Borrows the complete buffer for the duration of `f` without copying it;
    /// `None` while the data is not set.
    ///
    /// The synchronization read lock remains held while `f` runs, so keep the
    /// closure short to avoid delaying incoming server updates.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    /// Use [`Self::read_with_header`] to read the header with the buffer.
    pub fn read<R>(&self, f: impl FnOnce(Option<&[T]>) -> R) -> R {
        f(self
            .inner
            .read()
            .as_ref()
            .map(|store| store.data.as_slice()))
    }

    /// Returns a copy of the header, or `None` while the data is not set.
    pub fn header(&self) -> Option<H> {
        self.inner.read().as_ref().map(|store| store.header.clone())
    }

    /// Borrows the header for the duration of `f`; `None` while the data is
    /// not set.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn read_header<R>(&self, f: impl FnOnce(Option<&H>) -> R) -> R {
        f(self.inner.read().as_ref().map(|store| &store.header))
    }

    /// Returns a copy of the buffer and the header that belongs to it, read
    /// under one read lock; `None` while the data is not set.
    ///
    /// Separate [`Self::get`] and [`Self::header`] calls can each see a
    /// different server update. Use [`Self::read_with_header`] to inspect a
    /// large buffer without cloning it.
    pub fn get_with_header(&self) -> Option<(Vec<T>, H)> {
        self.inner
            .read()
            .as_ref()
            .map(|store| (store.data.clone(), store.header.clone()))
    }

    /// Borrows the buffer and the header that belongs to it for the duration
    /// of `f`, under one read lock; `None` while the data is not set.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn read_with_header<R>(&self, f: impl FnOnce(Option<(&[T], &H)>) -> R) -> R {
        let inner = self.inner.read();
        f(inner
            .as_ref()
            .map(|store| (store.data.as_slice(), &store.header)))
    }

    fn decode_header(&self, update: HeaderUpdate, bytes: &[u8]) -> Result<Option<H>, String> {
        decode_header(self.header_type, update, bytes, Label("Data", &self.name))
    }

    fn not_set(&self, what: &str) -> String {
        format!("Data {} is not set for {what}", self.name)
    }

    fn set_all(
        &self,
        payload: &[u8],
        transport_type: TransportType,
        header: HeaderUpdate,
    ) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));
        let (header_bytes, data) = split_header(payload, header.len(), Label("Data", &self.name))?;
        element_count::<T>(data, "Data")?;
        let header = self.decode_header(header, header_bytes)?;
        self.save_data(Payload::Bytes(data), transport_type, header)
    }

    fn batch_start(&self, data: &[u8], elements_count: u64) -> Result<(), String> {
        let buffer = BatchBuffer::start(data, elements_count)?;
        self.buffer.lock().replace(buffer);
        Ok(())
    }

    fn batch(&self, data: &[u8]) -> Result<(), String> {
        match *self.buffer.lock() {
            Some(ref mut buffer) => buffer.append(data),
            None => Err(format!(
                "No header found for Data: {} when updating batch",
                self.name
            )),
        }
    }

    fn batch_end(
        &self,
        payload: &[u8],
        transport_type: TransportType,
        header: HeaderUpdate,
    ) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));

        let (header_bytes, data) = split_header(payload, header.len(), Label("Data", &self.name))?;
        match self.buffer.lock().take() {
            Some(buffer) => {
                let data = buffer.finish(data)?;
                let header = self.decode_header(header, header_bytes)?;
                self.save_data(Payload::Buffer(data), transport_type, header)
            }
            None => Err(format!(
                "No header found for Data: {} when updating batch end",
                self.name
            )),
        }
    }

    fn drain(
        &self,
        index: u64,
        count: u64,
        header: HeaderUpdate,
        payload: &[u8],
    ) -> Result<(), String> {
        self.sender.send(ChannelMessage::Ack(self.id));
        let header = self.decode_header(header, payload)?;

        let mut w = self.inner.write();
        let store = w.as_mut().ok_or_else(|| self.not_set("Drain"))?;
        let Some(range) = checked_range(index, count, store.data.len()) else {
            return Err(format!(
                "Drain range ({} to {}) exceeds current data size {}",
                index,
                index.saturating_add(count),
                store.data.len()
            ));
        };
        store.data.drain(range);
        if let Some(header) = header {
            store.header = header;
        }

        Ok(())
    }

    /// Empties the buffer of data that is set; only a set creates the data.
    fn clear(&self, header: HeaderUpdate, payload: &[u8]) -> Result<(), String> {
        let header = self.decode_header(header, payload)?;
        // A later change must not complete an older batch.
        self.buffer.lock().take();
        let mut w = self.inner.write();
        let store = w.as_mut().ok_or_else(|| self.not_set("Clear"))?;
        store.data.clear();
        if let Some(header) = header {
            store.header = header;
        }
        Ok(())
    }

    fn save_data(
        &self,
        data: Payload<'_, T>,
        transport_type: TransportType,
        header: Option<H>,
    ) -> Result<(), String> {
        match transport_type {
            TransportType::Set(count) => {
                check_count(&data, count, "Set")?;
                let data = data.into_vec();
                let mut w = self.inner.write();
                let previous = match w.as_mut() {
                    Some(store) => {
                        if let Some(header) = header {
                            store.header = header;
                        }
                        Some(std::mem::replace(&mut store.data, data))
                    }
                    None => {
                        let header =
                            created_header(self.header_type, header, Label("Data", &self.name))?;
                        *w = Some(Store { data, header });
                        None
                    }
                };
                // The replaced buffer is freed after the lock is released.
                drop(w);
                drop(previous);
            }
            TransportType::Add(count) => {
                check_count(&data, count, "Add")?;
                let mut w = self.inner.write();
                let store = w.as_mut().ok_or_else(|| self.not_set("Add"))?;
                data.append_to(&mut store.data);
                if let Some(header) = header {
                    store.header = header;
                }
            }
            TransportType::Replace(start, count) => {
                check_count(&data, count, "Replace")?;
                let mut w = self.inner.write();
                let store = w.as_mut().ok_or_else(|| self.not_set("Replace"))?;
                let Some(range) = checked_range(start, count, store.data.len()) else {
                    return Err(format!(
                        "Replace range ({} to {}) exceeds current data size {}",
                        start,
                        start.saturating_add(count),
                        store.data.len()
                    ));
                };
                data.write_to(&mut store.data[range]);
                if let Some(header) = header {
                    store.header = header;
                }
            }
        }
        Ok(())
    }
}

impl<T, H> UpdateData for Data<T, H>
where
    T: private::GetDataType + Sync + Send,
    H: HeaderValue,
{
    fn update_data(&self, message: DataMessage) -> Result<(), String> {
        // A batch changes nothing readers can see before its end.
        let changes = !matches!(message.0, DataHeader::StartBatch(..) | DataHeader::Batch(_));
        let result = self.apply(message);
        if changes && result.is_ok() {
            self.changed.mark();
        }
        result
    }

    fn disconnected(&self) {
        self.buffer.lock().take();
    }
}

#[allow(private_bounds)]
impl<T, H> Data<T, H>
where
    T: private::GetDataType + Sync + Send,
    H: HeaderValue,
{
    /// Applies one server message; a failed one changes nothing readers see.
    fn apply(&self, (header, data): DataMessage) -> Result<(), String> {
        match header {
            DataHeader::All(data_type, transport_type, _, header) => {
                if let Err(error) = check_data_type(self.data_type, data_type, &self.name) {
                    self.sender.send_ack(self.id);
                    return Err(error);
                }
                self.set_all(&data, transport_type, header)
            }
            DataHeader::StartBatch(count, _) => self.batch_start(&data, count),
            DataHeader::Batch(_) => self.batch(&data),
            DataHeader::End(data_type, transport_type, _, header) => {
                if let Err(error) = check_data_type(self.data_type, data_type, &self.name) {
                    self.sender.send_ack(self.id);
                    return Err(error);
                }
                self.batch_end(&data, transport_type, header)
            }
            DataHeader::Drain(index, count, header) => self.drain(index, count, header, &data),
            DataHeader::Clear(header) => self.clear(header, &data),
            DataHeader::Header(header) => {
                let header = decode_header_bytes(
                    self.header_type,
                    header,
                    &data,
                    Label("Data", &self.name),
                )?;
                let mut w = self.inner.write();
                let store = w.as_mut().ok_or_else(|| self.not_set("Header"))?;
                store.header = header;
                Ok(())
            }
            DataHeader::Reset => {
                let previous = self.inner.write().take();
                self.buffer.lock().take();
                drop(previous);
                Ok(())
            }
        }
    }
}

impl<T, H> Clone for Data<T, H> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            header_type: self.header_type,
            inner: self.inner.clone(),
            buffer: self.buffer.clone(),
            sender: self.sender.clone(),
            changed: self.changed.clone(),
        }
    }
}

// MultiData -------------------------------------------------------------------
pub(crate) trait UpdateMultiData: Sync + Send {
    fn update(&self, key: u32, message: DataMessage) -> Result<(), String>;
    fn remove(&self, key: u32);
    fn reset(&self);
    /// Drops every partial transfer, which can never complete once its
    /// connection is closed.
    fn disconnected(&self);
}

/// The buffers of a [`DataMulti`] and their headers; every key has both.
struct MultiStore<T, H> {
    data: NoHashMap<u32, Vec<T>>,
    headers: NoHashMap<u32, H>,
    /// Whether the server changed a key since the UI took its flag. A key
    /// keeps its flag after it is removed, so the removal is reported.
    changed: NoHashMap<u32, AtomicBool>,
}

impl<T, H: HeaderValue> MultiStore<T, H> {
    /// Replaces the header of an existing `key` when one is given.
    fn set_header(&mut self, key: u32, header: Option<H>) {
        if let Some(header) = header {
            self.headers.insert(key, header);
        }
    }
}

/// A read-only client collection of server-controlled numeric buffers, each
/// with an optional typed header `H`.
///
/// Each buffer is indexed by a `u32` key and updated independently. Missing
/// keys return `None`; a key has a header from the change that creates it.
/// Complete batches become visible atomically, together with their header.
pub struct DataMulti<T, H = ()> {
    name: Arc<String>,
    id: u64,
    data_type: DataType,
    element_size: usize,
    header_type: Option<u32>,
    inner: Arc<RwLock<MultiStore<T, H>>>,
    buffers: Arc<Mutex<NoHashMap<u32, BatchBuffer<T>>>>,
    sender: MessageSender,
    any_changed: ChangedFlag,
}

#[allow(private_bounds)]
impl<T, H> DataMulti<T, H>
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
            inner: Arc::new(RwLock::new(MultiStore {
                data: NoHashMap::default(),
                headers: NoHashMap::default(),
                changed: NoHashMap::default(),
            })),
            buffers: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
            any_changed: ChangedFlag::default(),
        }
    }

    /// Tells whether the server changed, created or removed the buffer at
    /// `key`, or its header, since the last call for `key`, and clears the
    /// flag of `key`.
    ///
    /// Only this call clears it; reading does not, and the flags of the other
    /// keys and of [`Self::take_any_changed`] are separate. Call it once per
    /// frame and key, before reading, and use the result wherever the UI
    /// rebuilds something from that buffer: a second call in the same frame
    /// returns `false`. After a removal it returns `true` once, and reading
    /// the key returns `None`. The synchronization after a connection also
    /// counts as a change.
    ///
    /// ```no_run
    /// # fn downsample(samples: &[f32]) -> Vec<f32> { samples.to_vec() }
    /// # fn example(channels: egui_states::DataMulti<f32>, plots: &mut [Vec<f32>; 4]) {
    /// for (key, plot) in (0..).zip(plots.iter_mut()) {
    ///     if channels.take_changed(key) {
    ///         *plot = channels.read(key, |samples| samples.map(downsample).unwrap_or_default());
    ///     }
    /// }
    /// # }
    /// ```
    pub fn take_changed(&self, key: u32) -> bool {
        let inner = self.inner.read();
        inner
            .changed
            .get(&key)
            .is_some_and(|flag| flag.swap(false, Ordering::AcqRel))
    }

    /// Tells whether the server changed, created or removed any buffer or
    /// header since the last call, and clears this flag.
    ///
    /// It is separate from the flags of [`Self::take_changed`], for UI that
    /// rebuilds from the whole collection, such as with [`Self::read_all`].
    pub fn take_any_changed(&self) -> bool {
        self.any_changed.take()
    }

    /// Records a server change of `key` that readers can already see.
    fn mark(&self, key: u32) {
        let inner = self.inner.read();
        match inner.changed.get(&key) {
            Some(flag) => flag.store(true, Ordering::Release),
            None => {
                drop(inner);
                let flag = AtomicBool::new(true);
                self.inner.write().changed.insert(key, flag);
            }
        }
        self.any_changed.mark();
    }

    /// Returns a copy of the buffer at `key`, or `None` when it is absent.
    ///
    /// Use [`Self::get_with_header`] to copy the header with the buffer.
    #[inline]
    pub fn get(&self, key: u32) -> Option<Vec<T>> {
        self.inner.read().data.get(&key).cloned()
    }

    /// Returns the number of keys that hold a buffer, not the highest key
    /// plus one.
    pub fn len(&self) -> usize {
        self.inner.read().data.len()
    }

    /// Returns whether no key holds a buffer.
    pub fn is_empty(&self) -> bool {
        self.inner.read().data.is_empty()
    }

    /// Returns whether `key` holds a buffer.
    pub fn contains(&self, key: u32) -> bool {
        self.inner.read().data.contains_key(&key)
    }

    /// Returns a snapshot of the keys that hold a buffer, in ascending order.
    pub fn indices(&self) -> Vec<u32> {
        let mut indices = self.inner.read().data.keys().copied().collect::<Vec<_>>();
        indices.sort_unstable();
        indices
    }

    /// Borrows the buffer at `key` for the duration of `f`.
    ///
    /// The synchronization read lock remains held while `f` runs.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    /// Use [`Self::read_with_header`] to read the header with the buffer.
    #[inline]
    pub fn read<R>(&self, key: u32, f: impl FnOnce(Option<&[T]>) -> R) -> R {
        f(self.inner.read().data.get(&key).map(Vec::as_slice))
    }

    /// Borrows the complete keyed collection for the duration of `f`.
    ///
    /// The synchronization read lock remains held while `f` runs.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    /// Use [`Self::read_all_with_header`] to read the headers as well.
    #[inline]
    pub fn read_all<R>(&self, f: impl FnOnce(&NoHashMap<u32, Vec<T>>) -> R) -> R {
        f(&self.inner.read().data)
    }

    /// Borrows the complete keyed collection and the headers by key for the
    /// duration of `f`, under one read lock; both maps have the same keys.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn read_all_with_header<R>(
        &self,
        f: impl FnOnce(&NoHashMap<u32, Vec<T>>, &NoHashMap<u32, H>) -> R,
    ) -> R {
        let inner = self.inner.read();
        f(&inner.data, &inner.headers)
    }

    /// Calls `f` for every currently stored key and buffer.
    ///
    /// The synchronization read lock remains held for the full iteration.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    /// Use [`Self::for_each_with_header`] to read the headers as well.
    #[inline]
    pub fn for_each(&self, mut f: impl FnMut(u32, &[T])) {
        self.inner.read().data.iter().for_each(|(k, v)| f(*k, v));
    }

    /// Returns a copy of the header at `key`, or `None` when the key is
    /// absent.
    pub fn header(&self, key: u32) -> Option<H> {
        self.inner.read().headers.get(&key).cloned()
    }

    /// Borrows the header at `key` for the duration of `f`; `None` when the
    /// key is absent.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn read_header<R>(&self, key: u32, f: impl FnOnce(Option<&H>) -> R) -> R {
        f(self.inner.read().headers.get(&key))
    }

    /// Returns a copy of the buffer at `key` and its header, read under one
    /// read lock; `None` when the key is absent.
    ///
    /// Separate [`Self::get`] and [`Self::header`] calls can each see a
    /// different server update. Use [`Self::read_with_header`] to inspect a
    /// large buffer without cloning it.
    pub fn get_with_header(&self, key: u32) -> Option<(Vec<T>, H)> {
        let inner = self.inner.read();
        inner
            .data
            .get(&key)
            .zip(inner.headers.get(&key))
            .map(|(data, header)| (data.clone(), header.clone()))
    }

    /// Borrows the buffer at `key` and its header for the duration of `f`,
    /// under one read lock; `None` when the key is absent.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn read_with_header<R>(&self, key: u32, f: impl FnOnce(Option<(&[T], &H)>) -> R) -> R {
        let inner = self.inner.read();
        let entry = inner
            .data
            .get(&key)
            .zip(inner.headers.get(&key))
            .map(|(data, header)| (data.as_slice(), header));
        f(entry)
    }

    /// Calls `f` for every currently stored key, buffer and header.
    ///
    /// The synchronization read lock remains held for the full iteration.
    ///
    /// Do not call this handle or a clone of it from `f`: the read lock is not
    /// reentrant, so a nested read waits behind an incoming server update,
    /// which waits for `f`.
    pub fn for_each_with_header(&self, mut f: impl FnMut(u32, &[T], &H)) {
        let inner = self.inner.read();
        for (key, data) in inner.data.iter() {
            if let Some(header) = inner.headers.get(key) {
                f(*key, data, header);
            }
        }
    }

    fn decode_header(&self, update: HeaderUpdate, bytes: &[u8]) -> Result<Option<H>, String> {
        decode_header(self.header_type, update, bytes, Label("Data", &self.name))
    }

    fn set_all(
        &self,
        key: u32,
        payload: &[u8],
        transport_type: TransportType,
        header: HeaderUpdate,
    ) -> Result<(), String> {
        self.sender.send_ack_key(self.id, key);
        let (header_bytes, data) = split_header(payload, header.len(), Label("Data", &self.name))?;
        element_count::<T>(data, "Data")?;
        let header = self.decode_header(header, header_bytes)?;
        self.save_data(key, Payload::Bytes(data), transport_type, header)
    }

    fn batch_start(&self, key: u32, data: &[u8], elements_count: u64) -> Result<(), String> {
        let buffer = BatchBuffer::start(data, elements_count)?;
        self.buffers.lock().insert(key, buffer);
        Ok(())
    }

    fn batch(&self, key: u32, data: &[u8]) -> Result<(), String> {
        match self.buffers.lock().get_mut(&key) {
            Some(buffer) => buffer.append(data),
            None => Err(format!(
                "No header found for Data: {} when updating batch",
                self.name
            )),
        }
    }

    fn batch_end(
        &self,
        key: u32,
        payload: &[u8],
        transport_type: TransportType,
        header: HeaderUpdate,
    ) -> Result<(), String> {
        self.sender.send_ack_key(self.id, key);

        let (header_bytes, data) = split_header(payload, header.len(), Label("Data", &self.name))?;
        match self.buffers.lock().remove(&key) {
            Some(buffer) => {
                let data = buffer.finish(data)?;
                let header = self.decode_header(header, header_bytes)?;
                self.save_data(key, Payload::Buffer(data), transport_type, header)
            }
            None => Err(format!(
                "No header found for Data: {} when updating batch end",
                self.name
            )),
        }
    }

    fn drain(
        &self,
        key: u32,
        index: u64,
        count: u64,
        header: HeaderUpdate,
        payload: &[u8],
    ) -> Result<(), String> {
        self.sender.send_ack_key(self.id, key);
        let header = self.decode_header(header, payload)?;

        let mut w = self.inner.write();
        if let Some(data) = w.data.get_mut(&key) {
            let Some(range) = checked_range(index, count, data.len()) else {
                return Err(format!(
                    "Drain range ({} to {}) exceeds current data size {}",
                    index,
                    index.saturating_add(count),
                    data.len()
                ));
            };
            data.drain(range);
            w.set_header(key, header);
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
        data: Payload<'_, T>,
        transport_type: TransportType,
        header: Option<H>,
    ) -> Result<(), String> {
        match transport_type {
            TransportType::Set(count) => {
                check_count(&data, count, "Set")?;
                let data = data.into_vec();
                let mut w = self.inner.write();
                let header = match header {
                    None if !w.headers.contains_key(&key) => Some(created_header(
                        self.header_type,
                        None,
                        Label("Data", &self.name),
                    )?),
                    header => header,
                };
                let previous = w.data.insert(key, data);
                w.set_header(key, header);
                // The replaced buffer is freed after the lock is released.
                drop(w);
                drop(previous);
            }
            TransportType::Add(count) => {
                check_count(&data, count, "Add")?;
                let mut w = self.inner.write();
                let Some(target) = w.data.get_mut(&key) else {
                    return Err(format!(
                        "Key {} does not exist for Add transport type in MultiData: {}",
                        key, self.name
                    ));
                };
                data.append_to(target);
                w.set_header(key, header);
            }
            TransportType::Replace(start, count) => {
                check_count(&data, count, "Replace")?;
                let mut w = self.inner.write();
                let Some(target) = w.data.get_mut(&key) else {
                    return Err(format!(
                        "Key {} does not exist for Replace transport type in MultiData: {}",
                        key, self.name
                    ));
                };
                let Some(range) = checked_range(start, count, target.len()) else {
                    return Err(format!(
                        "Replace range ({} to {}) exceeds current data size {} for key {} in MultiData: {}",
                        start,
                        start.saturating_add(count),
                        target.len(),
                        key,
                        self.name
                    ));
                };
                data.write_to(&mut target[range]);
                w.set_header(key, header);
            }
        }
        Ok(())
    }

    fn missing_key(&self, key: u32, what: &str) -> String {
        format!(
            "Key {} does not exist for {} in MultiData: {}",
            key, what, self.name
        )
    }
}

impl<T, H> UpdateMultiData for DataMulti<T, H>
where
    T: private::GetDataType + Sync + Send,
    H: HeaderValue,
{
    fn update(&self, index: u32, message: DataMessage) -> Result<(), String> {
        // A batch changes nothing readers can see before its end.
        let changes = !matches!(message.0, DataHeader::StartBatch(..) | DataHeader::Batch(_));
        let result = self.apply(index, message);
        if changes && result.is_ok() {
            self.mark(index);
        }
        result
    }

    fn remove(&self, index: u32) {
        let mut w = self.inner.write();
        w.data.remove(&index);
        w.headers.remove(&index);
        drop(w);
        self.buffers.lock().remove(&index);
        self.mark(index);
    }

    fn reset(&self) {
        let mut w = self.inner.write();
        w.data.clear();
        w.headers.clear();
        // Every key is removed; the flags stay to report it.
        for flag in w.changed.values() {
            flag.store(true, Ordering::Release);
        }
        drop(w);
        self.buffers.lock().clear();
        self.any_changed.mark();
    }

    fn disconnected(&self) {
        self.buffers.lock().clear();
    }
}

#[allow(private_bounds)]
impl<T, H> DataMulti<T, H>
where
    T: private::GetDataType + Sync + Send,
    H: HeaderValue,
{
    /// Applies one server message for `index`; a failed one changes nothing
    /// readers see.
    fn apply(&self, index: u32, (header, data): DataMessage) -> Result<(), String> {
        match header {
            DataHeader::All(data_type, transport_type, _, header) => {
                if let Err(error) = check_data_type(self.data_type, data_type, &self.name) {
                    self.sender.send_ack_key(self.id, index);
                    return Err(error);
                }
                self.set_all(index, &data, transport_type, header)
            }
            DataHeader::StartBatch(count, _) => self.batch_start(index, &data, count),
            DataHeader::Batch(_) => self.batch(index, &data),
            DataHeader::End(data_type, transport_type, _, header) => {
                if let Err(error) = check_data_type(self.data_type, data_type, &self.name) {
                    self.sender.send_ack_key(self.id, index);
                    return Err(error);
                }
                self.batch_end(index, &data, transport_type, header)
            }
            DataHeader::Drain(start, count, header) => {
                self.drain(index, start, count, header, &data)
            }
            DataHeader::Clear(header_update) => {
                let header = self.decode_header(header_update, &data)?;
                // A later change of this key must not complete an older batch.
                self.buffers.lock().remove(&index);
                let mut w = self.inner.write();
                match w.data.get_mut(&index) {
                    Some(val) => {
                        val.clear();
                        w.set_header(index, header);
                        Ok(())
                    }
                    // A clear that keeps the header has nothing to lose.
                    None if header.is_none() => Ok(()),
                    None => Err(self.missing_key(index, "Clear")),
                }
            }
            DataHeader::Header(header) => {
                let header = decode_header_bytes(
                    self.header_type,
                    header,
                    &data,
                    Label("Data", &self.name),
                )?;
                let mut w = self.inner.write();
                if !w.data.contains_key(&index) {
                    return Err(self.missing_key(index, "Header"));
                }
                w.set_header(index, Some(header));
                Ok(())
            }
            DataHeader::Reset => Err(format!(
                "A Data reset is not valid for key {} in MultiData: {}",
                index, self.name
            )),
        }
    }
}

impl<T, H> Clone for DataMulti<T, H> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            data_type: self.data_type,
            element_size: self.element_size,
            header_type: self.header_type,
            inner: self.inner.clone(),
            buffers: self.buffers.clone(),
            sender: self.sender.clone(),
            any_changed: self.any_changed.clone(),
        }
    }
}

// functions -------------------------------------------------------------------
#[inline]
pub(crate) fn check_data_type(
    expected: DataType,
    actual: DataType,
    name: &str,
) -> Result<(), String> {
    if expected != actual {
        return Err(format!(
            "Data type {:?} does not match expected type {:?} for Data: {}",
            actual, expected, name
        ));
    }
    Ok(())
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
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    /// Clear, remove, reset and a disconnect drop the partial batch of their
    /// key, so the batch end that follows fails with one acknowledgement.
    #[test]
    fn controls_and_disconnects_drop_partial_batches() {
        let id = 42;
        let start = || (DataHeader::StartBatch(4, 0), Bytes::from_static(&[1, 2]));
        let end = || {
            (
                DataHeader::End(
                    DataType::U8,
                    TransportType::Set(4),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep,
                ),
                Bytes::from_static(&[3, 4]),
            )
        };

        let (sender, mut receiver) = MessageSender::new();
        let single = Data::<u8>::new("data".to_string(), id, sender);
        single.update_data(start()).unwrap();
        single.disconnected();
        assert!(single.update_data(end()).is_err());
        assert_single_ack(&mut receiver, id);

        type Control = fn(&DataMulti<u8>);
        let controls: [(&str, Control); 4] = [
            ("clear", |data| {
                data.update(
                    1,
                    (
                        DataHeader::Clear(crate::protocol::data::HeaderUpdate::Keep),
                        Bytes::new(),
                    ),
                )
                .unwrap()
            }),
            ("remove", |data| data.remove(1)),
            ("reset", |data| data.reset()),
            ("disconnect", |data| data.disconnected()),
        ];
        for (name, control) in controls {
            let (sender, mut receiver) = MessageSender::new();
            let multi = DataMulti::<u8>::new("data".to_string(), id, sender);
            multi.update(1, start()).unwrap();
            control(&multi);
            assert!(multi.update(1, end()).is_err(), "{name} kept the batch");
            assert_single_ack(&mut receiver, id);
            assert_eq!(multi.get(1).map(|v| v.len()).unwrap_or(0), 0, "{name}");
        }
    }

    /// `for_each` is callable with a plain closure, without a turbofish.
    #[test]
    fn data_multi_for_each_visits_every_key() {
        let (sender, _receiver) = MessageSender::new();
        let data = DataMulti::<u8>::new("data".to_string(), 40, sender);
        for key in [3, 5] {
            data.update(
                key,
                (
                    DataHeader::All(
                        DataType::U8,
                        TransportType::Set(2),
                        0,
                        crate::protocol::data::HeaderUpdate::Keep,
                    ),
                    Bytes::from(vec![key as u8; 2]),
                ),
            )
            .unwrap();
        }
        let mut seen = Vec::new();
        data.for_each(|key, values| seen.push((key, values.to_vec())));
        seen.sort();
        assert_eq!(seen, [(3, vec![3, 3]), (5, vec![5, 5])]);
    }

    #[test]
    fn data_type_mismatch_acks_all_and_batch_end() {
        let id = 41;
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u8>::new("data".to_string(), id, sender);

        assert!(
            data.update_data((
                DataHeader::All(
                    DataType::U16,
                    TransportType::Set(1),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep
                ),
                Bytes::from_static(&[0, 0])
            ))
            .is_err()
        );
        assert_single_ack(&mut receiver, id);

        assert!(
            data.update_data((
                DataHeader::End(
                    DataType::U16,
                    TransportType::Set(1),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep
                ),
                Bytes::from_static(&[0, 0])
            ))
            .is_err()
        );
        assert_single_ack(&mut receiver, id);
    }

    #[test]
    fn data_multi_type_mismatch_acks_all_and_batch_end() {
        let id = 42;
        let (sender, mut receiver) = MessageSender::new();
        let data = DataMulti::<u8>::new("data_multi".to_string(), id, sender);

        assert!(
            data.update(
                7,
                (
                    DataHeader::All(
                        DataType::U16,
                        TransportType::Set(1),
                        0,
                        crate::protocol::data::HeaderUpdate::Keep
                    ),
                    Bytes::from_static(&[0, 0])
                ),
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);

        assert!(
            data.update(
                7,
                (
                    DataHeader::End(
                        DataType::U16,
                        TransportType::Set(1),
                        0,
                        crate::protocol::data::HeaderUpdate::Keep
                    ),
                    Bytes::from_static(&[0, 0])
                ),
            )
            .is_err()
        );
        assert_single_ack(&mut receiver, id);
    }
}

#[cfg(test)]
mod malformed_tests {
    use super::*;

    #[test]
    fn invalid_count_and_unfinished_batches_preserve_previous_data_and_recover() {
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u8>::new("buffer".into(), 91, sender);
        data.update_data((
            DataHeader::All(
                DataType::U8,
                TransportType::Set(2),
                0,
                crate::protocol::data::HeaderUpdate::Keep,
            ),
            Bytes::from_static(&[7, 8]),
        ))
        .unwrap();
        crate::test_support::ack(&mut receiver, 91);
        assert!(
            data.update_data((
                DataHeader::All(
                    DataType::U8,
                    TransportType::Set(3),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep
                ),
                Bytes::from_static(&[1])
            ))
            .is_err()
        );
        crate::test_support::ack(&mut receiver, 91);
        data.read(|v| assert_eq!(v, Some(&[7, 8][..])));
        data.update_data((DataHeader::StartBatch(3, 0), Bytes::from_static(&[1])))
            .unwrap();
        data.read(|v| assert_eq!(v, Some(&[7, 8][..])));
        assert!(receiver.try_recv().is_err());
        assert!(
            data.update_data((
                DataHeader::End(
                    DataType::U8,
                    TransportType::Set(3),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep
                ),
                Bytes::from_static(&[2])
            ))
            .is_err()
        );
        crate::test_support::ack(&mut receiver, 91);
        data.read(|v| assert_eq!(v, Some(&[7, 8][..])));
        data.update_data((
            DataHeader::All(
                DataType::U8,
                TransportType::Set(1),
                0,
                crate::protocol::data::HeaderUpdate::Keep,
            ),
            Bytes::from_static(&[9]),
        ))
        .unwrap();
        crate::test_support::ack(&mut receiver, 91);
        data.read(|v| assert_eq!(v, Some(&[9][..])));
    }

    fn set(data: &Data<u8>, values: &[u8]) {
        data.update_data((
            DataHeader::All(
                DataType::U8,
                TransportType::Set(values.len() as u64),
                0,
                crate::protocol::data::HeaderUpdate::Keep,
            ),
            Bytes::copy_from_slice(values),
        ))
        .unwrap();
    }

    fn set_key(data: &DataMulti<u8>, key: u32, values: &[u8]) {
        data.update(
            key,
            (
                DataHeader::All(
                    DataType::U8,
                    TransportType::Set(values.len() as u64),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep,
                ),
                Bytes::copy_from_slice(values),
            ),
        )
        .unwrap();
    }

    /// The flag reports each change readers can see once, is cleared only by
    /// taking it, and is shared by clones.
    #[test]
    fn take_changed_reports_each_visible_change_once() {
        let (sender, _receiver) = MessageSender::new();
        let data = Data::<u8>::new("data".to_string(), 1, sender);
        assert!(!data.take_changed());

        set(&data, &[1, 2]);
        assert!(data.take_changed());
        assert!(!data.take_changed());

        // Reading leaves the flag alone.
        set(&data, &[3]);
        assert_eq!(data.get(), Some(vec![3]));
        data.read(|_| ());
        assert!(data.clone().take_changed());
        assert!(!data.take_changed());

        // A batch is visible, and reported, only at its end.
        data.update_data((DataHeader::StartBatch(2, 0), Bytes::from_static(&[4])))
            .unwrap();
        data.update_data((DataHeader::Batch(0), Bytes::new()))
            .unwrap();
        assert!(!data.take_changed());
        data.update_data((
            DataHeader::End(
                DataType::U8,
                TransportType::Set(2),
                0,
                crate::protocol::data::HeaderUpdate::Keep,
            ),
            Bytes::from_static(&[5]),
        ))
        .unwrap();
        assert!(data.take_changed());

        // A failed update changes nothing.
        let drain = DataHeader::Drain(5, 1, crate::protocol::data::HeaderUpdate::Keep);
        assert!(data.update_data((drain, Bytes::new())).is_err());
        assert!(!data.take_changed());

        data.update_data((DataHeader::Reset, Bytes::new())).unwrap();
        assert!(data.take_changed());
        assert_eq!(data.get(), None);
    }

    /// Every key has its own flag, which also reports a removal, and one more
    /// flag reports a change of any key.
    #[test]
    fn data_multi_flags_changes_per_key_and_for_any_key() {
        let (sender, _receiver) = MessageSender::new();
        let data = DataMulti::<u8>::new("data".to_string(), 2, sender);
        assert!(!data.take_changed(1));
        assert!(!data.take_any_changed());

        set_key(&data, 1, &[1]);
        assert!(!data.take_changed(2));
        assert!(data.take_changed(1));
        assert!(!data.take_changed(1));
        assert!(data.take_any_changed());
        assert!(!data.take_any_changed());

        set_key(&data, 2, &[2]);
        assert!(!data.take_changed(1));
        assert!(data.take_changed(2));
        assert!(data.take_any_changed());

        // Reading leaves the flags alone; a batch counts at its end.
        set_key(&data, 1, &[3]);
        assert_eq!(data.get(1), Some(vec![3]));
        data.read_all(|_| ());
        data.update(1, (DataHeader::StartBatch(2, 0), Bytes::from_static(&[4])))
            .unwrap();
        assert!(data.clone().take_changed(1));
        data.update(1, (DataHeader::Batch(0), Bytes::new()))
            .unwrap();
        assert!(!data.take_changed(1));

        data.remove(1);
        assert!(data.take_changed(1));
        assert_eq!(data.get(1), None);
        assert!(!data.take_changed(1));
        assert!(data.take_any_changed());

        data.reset();
        assert!(data.take_changed(1));
        assert!(data.take_changed(2));
        assert!(data.take_any_changed());
        assert!(!data.take_changed(3));
    }

    #[test]
    fn data_multi_acknowledges_each_key_on_its_own() {
        let (sender, mut receiver) = MessageSender::new();
        let data = DataMulti::<u8>::new("data".to_string(), 4, sender);
        set_key(&data, 7, &[1]);
        set_key(&data, 2, &[2]);
        for key in [7, 2] {
            assert!(matches!(
                receiver.try_recv(),
                Ok(Some(ChannelMessage::AckKey(4, acked))) if acked == key
            ));
        }
    }

    #[test]
    fn data_multi_lists_the_keys_that_hold_a_buffer() {
        let (sender, _receiver) = MessageSender::new();
        let data = DataMulti::<u8>::new("data".to_string(), 3, sender);
        assert!(data.is_empty());
        assert_eq!(data.indices(), Vec::<u32>::new());

        set_key(&data, 7, &[1]);
        set_key(&data, 2, &[]);
        // A batch still arriving is not a key yet.
        data.update(5, (DataHeader::StartBatch(2, 0), Bytes::from_static(&[4])))
            .unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data.indices(), vec![2, 7]);
        assert!(data.contains(2));
        assert!(!data.contains(5));

        data.remove(7);
        assert_eq!(data.indices(), vec![2]);
        assert!(!data.contains(7));
        data.reset();
        assert!(data.is_empty());
    }

    #[test]
    fn batch_chunks_without_a_start_are_rejected() {
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u8>::new("buffer".into(), 92, sender);
        set(&data, &[7, 8]);
        crate::test_support::ack(&mut receiver, 92);

        assert!(
            data.update_data((DataHeader::Batch(0), Bytes::from_static(&[1])))
                .is_err()
        );
        assert!(
            receiver.try_recv().is_err(),
            "a middle chunk is never acknowledged"
        );
        assert!(
            data.update_data((
                DataHeader::End(
                    DataType::U8,
                    TransportType::Set(1),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep
                ),
                Bytes::from_static(&[1])
            ))
            .is_err()
        );
        // The server waits for the end of every transfer, failed or not.
        crate::test_support::ack(&mut receiver, 92);
        data.read(|v| assert_eq!(v, Some(&[7, 8][..])));
    }

    #[test]
    fn a_new_batch_start_discards_a_stale_partial_batch() {
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u8>::new("buffer".into(), 93, sender);
        data.update_data((DataHeader::StartBatch(3, 0), Bytes::from_static(&[1])))
            .unwrap();
        data.update_data((DataHeader::StartBatch(2, 0), Bytes::from_static(&[5])))
            .unwrap();
        data.update_data((
            DataHeader::End(
                DataType::U8,
                TransportType::Set(2),
                0,
                crate::protocol::data::HeaderUpdate::Keep,
            ),
            Bytes::from_static(&[6]),
        ))
        .unwrap();
        crate::test_support::ack(&mut receiver, 93);
        data.read(|v| assert_eq!(v, Some(&[5, 6][..])));
    }

    #[test]
    fn out_of_range_drain_keeps_data_and_still_acknowledges() {
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u8>::new("buffer".into(), 94, sender);
        set(&data, &[1, 2, 3]);
        crate::test_support::ack(&mut receiver, 94);

        assert!(
            data.update_data((
                DataHeader::Drain(2, 5, crate::protocol::data::HeaderUpdate::Keep),
                Bytes::new()
            ))
            .is_err()
        );
        crate::test_support::ack(&mut receiver, 94);
        data.read(|v| assert_eq!(v, Some(&[1, 2, 3][..])));

        data.update_data((
            DataHeader::Drain(0, 1, crate::protocol::data::HeaderUpdate::Keep),
            Bytes::new(),
        ))
        .unwrap();
        crate::test_support::ack(&mut receiver, 94);
        data.read(|v| assert_eq!(v, Some(&[2, 3][..])));
    }

    /// Counts and ranges come from the network, so absurd values must be
    /// errors rather than panics, aborts, or short buffers.
    #[test]
    fn oversized_wire_counts_and_ranges_are_rejected() {
        let (sender, _receiver) = MessageSender::new();
        let data = Data::<u16>::new("buffer".into(), 95, sender.clone());
        let multi = DataMulti::<u16>::new("buffers".into(), 95, sender);
        let values = || {
            (
                DataHeader::All(
                    DataType::U16,
                    TransportType::Set(3),
                    0,
                    crate::protocol::data::HeaderUpdate::Keep,
                ),
                Bytes::from_static(&[1, 0, 2, 0, 3, 0]),
            )
        };
        data.update_data(values()).unwrap();
        multi.update(4, values()).unwrap();

        for count in [u64::MAX, u64::MAX / 2, 1 << 62] {
            let start = || {
                (
                    DataHeader::StartBatch(count, 0),
                    Bytes::from_static(&[1, 0]),
                )
            };
            assert!(data.update_data(start()).is_err());
            assert!(multi.update(4, start()).is_err());
        }
        for start in [u64::MAX, u64::MAX - 1] {
            let drain = || {
                (
                    DataHeader::Drain(start, 2, crate::protocol::data::HeaderUpdate::Keep),
                    Bytes::new(),
                )
            };
            assert!(data.update_data(drain()).is_err());
            assert!(multi.update(4, drain()).is_err());
            let replace = || {
                (
                    DataHeader::All(
                        DataType::U16,
                        TransportType::Replace(start, 1),
                        0,
                        crate::protocol::data::HeaderUpdate::Keep,
                    ),
                    Bytes::from_static(&[9, 0]),
                )
            };
            assert!(data.update_data(replace()).is_err());
            assert!(multi.update(4, replace()).is_err());
        }
        data.read(|v| assert_eq!(v, Some(&[1, 2, 3][..])));
        multi.read(4, |v| assert_eq!(v, Some(&[1, 2, 3][..])));
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;
    use crate::client::messages::{ChannelMessage, MessagesParser};
    use crate::protocol::data::HeaderBytes;
    use crate::protocol::{ServerHeader, VALUE_MAX_SIZE};
    use crate::typed::Typed;
    use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

    type Meta = (u32, String);

    fn meta(n: u32) -> Meta {
        (n, format!("m{n}"))
    }

    fn type_id() -> u32 {
        <Meta as Typed>::get_type().get_hash()
    }

    /// A payload of `header` followed by `values`, and its header update.
    fn payload(header: &Meta, type_id: u32, values: &[u16]) -> (HeaderUpdate, Bytes) {
        let mut bytes = postcard::to_stdvec(header).unwrap();
        let update = HeaderUpdate::Set(HeaderBytes {
            type_id,
            size: bytes.len() as u32,
        });
        bytes.extend(values.iter().flat_map(|v| v.to_ne_bytes()));
        (update, Bytes::from(bytes))
    }

    fn set(update: HeaderUpdate, values: &[u16]) -> DataHeader {
        let count = values.len() as u64;
        let len = size_of_val(values) as u32;
        DataHeader::All(DataType::U16, TransportType::Set(count), len, update)
    }

    fn acks(receiver: &mut UnboundedReceiver<Option<ChannelMessage>>) -> usize {
        let mut count = 0;
        while let Ok(Some(ChannelMessage::Ack(_) | ChannelMessage::AckKey(..))) =
            receiver.try_recv()
        {
            count += 1;
        }
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        count
    }

    #[test]
    fn a_bad_header_leaves_buffer_and_header_unchanged() {
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u16, Meta>::new("root.data".into(), 7, sender.clone());
        let (update, bytes) = payload(&meta(1), type_id(), &[1, 2]);
        data.update_data((set(update, &[1, 2]), bytes)).unwrap();
        assert_eq!(acks(&mut receiver), 1);

        let (update, bytes) = payload(&meta(2), type_id() ^ 1, &[3]);
        let error = data.update_data((set(update, &[3]), bytes)).unwrap_err();
        assert!(error.contains("type id"), "{error}");
        assert_eq!(acks(&mut receiver), 1);

        // Too short for the header's string.
        let (HeaderUpdate::Set(mut header), bytes) = payload(&meta(3), type_id(), &[]) else {
            unreachable!()
        };
        header.size -= 1;
        let corrupt = bytes.slice(..header.size as usize);
        let error = data
            .update_data((set(HeaderUpdate::Set(header), &[]), corrupt))
            .unwrap_err();
        assert!(error.contains("Header parse error"), "{error}");
        assert_eq!(acks(&mut receiver), 1);

        let oversized = HeaderUpdate::Set(HeaderBytes {
            type_id: type_id(),
            size: VALUE_MAX_SIZE as u32 + 1,
        });
        let bytes = Bytes::from(vec![0; VALUE_MAX_SIZE + 1]);
        let error = data.update_data((set(oversized, &[]), bytes)).unwrap_err();
        assert!(error.contains("too large"), "{error}");
        assert_eq!(acks(&mut receiver), 1);

        assert_eq!(
            (data.get(), data.header()),
            (Some(vec![1, 2]), Some(meta(1)))
        );

        // A state without a header refuses one.
        let plain = Data::<u16>::new("root.plain".into(), 8, sender);
        let (update, bytes) = payload(&meta(1), type_id(), &[1]);
        let error = plain.update_data((set(update, &[1]), bytes)).unwrap_err();
        assert!(error.contains("has no header"), "{error}");
        assert_eq!(acks(&mut receiver), 1);
        assert_eq!(plain.get(), None);
    }

    #[test]
    fn a_batch_header_arrives_with_its_end() {
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u16, Meta>::new("root.data".into(), 7, sender);
        data.update_data((DataHeader::StartBatch(3, 2), Bytes::from_static(&[1, 0])))
            .unwrap();
        data.update_data((DataHeader::Batch(2), Bytes::from_static(&[2, 0])))
            .unwrap();
        assert_eq!(data.get_with_header(), None);

        let (update, bytes) = payload(&meta(5), type_id(), &[3]);
        let end = DataHeader::End(DataType::U16, TransportType::Set(3), 2, update);
        data.update_data((end, bytes)).unwrap();
        assert_eq!(acks(&mut receiver), 1);
        data.read_with_header(|entry| {
            assert_eq!(entry, Some((&[1, 2, 3][..], &meta(5))));
        });
    }

    #[test]
    fn header_only_messages_need_an_existing_key() {
        let (sender, mut receiver) = MessageSender::new();
        let multi = DataMulti::<u16, Meta>::new("root.multi".into(), 7, sender);
        let (update, bytes) = payload(&meta(1), type_id(), &[]);
        let HeaderUpdate::Set(header) = update else {
            unreachable!()
        };

        let error = multi
            .update(4, (DataHeader::Header(header), bytes.clone()))
            .unwrap_err();
        assert!(error.contains("does not exist"), "{error}");
        let error = multi
            .update(4, (DataHeader::Clear(update), bytes.clone()))
            .unwrap_err();
        assert!(error.contains("does not exist"), "{error}");
        // A clear that keeps the header loses nothing on a missing key.
        multi
            .update(4, (DataHeader::Clear(HeaderUpdate::Keep), Bytes::new()))
            .unwrap();
        assert_eq!(multi.header(4), None);

        // A new key needs a header, and only a Data takes a reset.
        let (keep, values) = (HeaderUpdate::Keep, Bytes::from_static(&[9, 0]));
        let error = multi.update(4, (set(keep, &[9]), values)).unwrap_err();
        assert!(error.contains("without a header"), "{error}");
        assert_eq!(multi.get(4), None);
        let error = multi
            .update(4, (DataHeader::Reset, Bytes::new()))
            .unwrap_err();
        assert!(error.contains("reset"), "{error}");

        let (created, values) = payload(&meta(0), type_id(), &[9]);
        multi.update(4, (set(created, &[9]), values)).unwrap();
        assert_eq!(multi.header(4), Some(meta(0)));
        multi
            .update(4, (DataHeader::Header(header), bytes))
            .unwrap();
        assert_eq!(multi.header(4), Some(meta(1)));
        assert_eq!(multi.get(4), Some(vec![9]));
        // Only the sets were acknowledged.
        assert_eq!(acks(&mut receiver), 2);
    }

    #[test]
    fn data_is_not_set_until_a_set_creates_it() {
        let (sender, mut receiver) = MessageSender::new();
        let data = Data::<u16, Meta>::new("root.data".into(), 7, sender.clone());
        assert_eq!(data.get_with_header(), None);
        data.read(|values| assert_eq!(values, None));
        data.read_header(|header| assert_eq!(header, None));

        // A change other than a set needs data that is set.
        let (update, bytes) = payload(&meta(1), type_id(), &[1]);
        let add = DataHeader::All(DataType::U16, TransportType::Add(1), 2, update);
        let error = data.update_data((add, bytes)).unwrap_err();
        assert!(error.contains("is not set"), "{error}");
        let (_, header_bytes) = payload(&meta(1), type_id(), &[]);
        let HeaderUpdate::Set(header) = update else {
            unreachable!()
        };
        let error = data
            .update_data((DataHeader::Header(header), header_bytes.clone()))
            .unwrap_err();
        assert!(error.contains("is not set"), "{error}");
        let drain = DataHeader::Drain(0, 0, HeaderUpdate::Keep);
        let error = data.update_data((drain, Bytes::new())).unwrap_err();
        assert!(error.contains("is not set"), "{error}");

        // With a header type, a set needs a header to create the data.
        let keep = set(HeaderUpdate::Keep, &[1]);
        let error = data
            .update_data((keep, Bytes::from_static(&[1, 0])))
            .unwrap_err();
        assert!(error.contains("without a header"), "{error}");
        assert_eq!(data.get(), None);
        assert_eq!(acks(&mut receiver), 3);

        // A clear does not create the data, with a header or without one.
        for clear in [update, HeaderUpdate::Keep] {
            let bytes = match clear {
                HeaderUpdate::Keep => Bytes::new(),
                HeaderUpdate::Set(_) => header_bytes.clone(),
            };
            let error = data
                .update_data((DataHeader::Clear(clear), bytes))
                .unwrap_err();
            assert!(error.contains("is not set"), "{error}");
            assert_eq!(data.get_with_header(), None);
        }

        // The synchronization of an empty buffer is a set with its header.
        data.update_data((set(update, &[]), header_bytes)).unwrap();
        assert_eq!(data.get_with_header(), Some((vec![], meta(1))));
        assert_eq!(acks(&mut receiver), 1);
        data.update_data((DataHeader::Reset, Bytes::new())).unwrap();
        assert_eq!(data.get_with_header(), None);
        assert_eq!(acks(&mut receiver), 0);

        // Without a header type, a set creates the data alone.
        let plain = Data::<u16>::new("root.plain".into(), 8, sender);
        let clear = DataHeader::Clear(HeaderUpdate::Keep);
        let error = plain.update_data((clear, Bytes::new())).unwrap_err();
        assert!(error.contains("is not set"), "{error}");
        plain
            .update_data((set(HeaderUpdate::Keep, &[]), Bytes::new()))
            .unwrap();
        assert_eq!(plain.get_with_header(), Some((vec![], ())));
    }

    #[test]
    fn only_a_header_changes_the_layout_hash() {
        use private::GetDataType;

        assert_eq!(data_type_hash::<u16, ()>(), u16::get_type_id());
        assert_ne!(data_type_hash::<u16, Meta>(), u16::get_type_id());
        assert_ne!(data_type_hash::<u16, Meta>(), data_type_hash::<u32, Meta>());
        assert_eq!(header_type_id::<()>(), None);
        assert_eq!(header_type_id::<Meta>(), Some(type_id()));
    }

    #[test]
    fn header_bytes_outside_the_frame_end_the_connection() {
        let header = HeaderBytes {
            type_id: type_id(),
            size: 10,
        };
        let message = ServerHeader::Data(
            7,
            DataHeader::All(
                DataType::U8,
                TransportType::Set(0),
                0,
                HeaderUpdate::Set(header),
            ),
        );
        let mut frame = postcard::to_stdvec(&message).unwrap();
        frame.extend([1, 2, 3]);
        assert!(MessagesParser::from_bytes(Bytes::from(frame)).is_err());

        let message = ServerHeader::Data(7, DataHeader::Header(header));
        let mut frame = postcard::to_stdvec(&message).unwrap();
        frame.extend([0; 10]);
        let (_, (_, payload)) = MessagesParser::from_bytes(Bytes::from(frame)).unwrap();
        assert_eq!(payload.len(), 10);
    }
}
