//! Server handles for contiguous and one-shot numeric buffers.

use std::marker::PhantomData;
use std::sync::Arc;

use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::protocol::data::{DataType, header_type_id};
use crate::server_core::states::data::{Data as CoreData, DataMulti as CoreDataMulti, DataSlice};
use crate::server_core::states::data_take::{
    DataMultiTake as CoreDataMultiTake, DataTake as CoreDataTake,
};
use crate::typed::Typed;

use super::state_server::{StateServer, deserialize_bytes, serialize_bytes};
use super::{Result, ServerError};

/// The header type of a Data state: any `Typed` value sent with the buffer,
/// `()` (the default) for a state without one.
///
/// It must match the client's header type. A header that was never set reads
/// as `None` here, while the client shows `H::default()`; once set, a
/// [`Data`] header cannot return to "never set". A custom [`Typed`] type
/// whose [`Typed::get_type`] is [`crate::ObjectType::Empty`] also means no
/// header.
pub trait HeaderValue: Typed + Serialize + DeserializeOwned + Send + Sync + 'static {}

impl<H> HeaderValue for H where H: Typed + Serialize + DeserializeOwned + Send + Sync + 'static {}

fn header_type<H: HeaderValue>() -> Option<u32> {
    header_type_id(&H::get_type())
}

/// `data` with its encoded `header`, for the cores.
fn with_header<'a, T: DataElement>(data: &'a [T], header: Option<&'a Bytes>) -> DataSlice<'a> {
    DataSlice {
        header: header.map(|header| header.as_ref()),
        ..data_slice(data)
    }
}

fn decode_header<H: HeaderValue>(header: Option<&[u8]>) -> Result<Option<H>> {
    header.map(deserialize_bytes).transpose()
}

/// A numeric element that can be transported by a [`Data`] state.
///
/// Built-in implementations are provided for `u8` through `u64`, `i8` through
/// `i64`, `f32`, and `f64`. Bytes are transported in native-endian form.
///
/// # Safety
///
/// `TYPE_ID` must identify a protocol data type with exactly the same size as
/// `Self`. Every bit pattern received for that protocol type must also be a
/// valid value of `Self`, and `Self` must not contain padding bytes.
pub unsafe trait DataElement: Copy + Send + Sync + 'static {
    /// Numeric protocol type identifier for `Self`.
    const TYPE_ID: u8;
}

macro_rules! impl_data_element {
    ($(($ty:ty, $variant:ident)),* $(,)?) => {
        $(
            unsafe impl DataElement for $ty {
                const TYPE_ID: u8 = DataType::$variant.id();
            }
        )*
    };
}

impl_data_element! {
    (u8, U8),
    (u16, U16),
    (u32, U32),
    (u64, U64),
    (i8, I8),
    (i16, I16),
    (i32, I32),
    (i64, I64),
    (f32, F32),
    (f64, F64),
}

fn data_type<T: DataElement>() -> DataType {
    DataType::from_id(T::TYPE_ID).expect("DataElement type id must be valid")
}

fn data_slice<T: DataElement>(data: &[T]) -> DataSlice<'_> {
    // SAFETY: `DataElement` types have no padding, so their values are plain bytes.
    let bytes = unsafe {
        std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(data))
    };
    DataSlice {
        bytes,
        data_type: data_type::<T>(),
        header: None,
    }
}

fn data_from_bytes<T: DataElement>(data: &[u8]) -> Result<Vec<T>> {
    let size = std::mem::size_of::<T>();
    if size == 0 || !data.len().is_multiple_of(size) {
        return Err(ServerError::new("invalid data byte size"));
    }

    let count = data.len() / size;
    let mut result = Vec::<T>::with_capacity(count);
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), result.as_mut_ptr() as *mut u8, data.len());
        result.set_len(count);
    }
    Ok(result)
}

/// Server-side contiguous numeric buffer mirrored on the client, with an
/// optional typed header `H`.
///
/// The methods without a header keep the current one. A header sent with a
/// change reaches the client together with the buffer.
pub struct Data<T, H = ()> {
    inner: Arc<CoreData>,
    _type: PhantomData<fn() -> (T, H)>,
}

impl<T, H> Clone for Data<T, H> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _type: PhantomData,
        }
    }
}

impl<T, H> Data<T, H>
where
    T: DataElement,
    H: HeaderValue,
{
    /// Registers an empty numeric buffer under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data::<T>(name.into(), header_type::<H>())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    /// Returns a copy of the complete buffer.
    pub fn get(&self) -> Result<Vec<T>> {
        self.inner.get(data_from_bytes)
    }

    /// Passes the typed buffer to `f`.
    pub fn read<R>(&self, f: impl FnOnce(&[T]) -> R) -> Result<R> {
        let data = self.get()?;
        Ok(f(&data))
    }

    /// Borrows the buffer's native-endian byte representation for `f`.
    pub fn read_bytes<R>(&self, f: impl Fn(&[u8]) -> R) -> R {
        self.inner.get(f)
    }

    /// Returns the header, or `None` until one is set.
    pub fn header(&self) -> Result<Option<H>> {
        self.inner.header(decode_header)
    }

    /// Replaces the complete buffer and optionally requests a client repaint.
    pub fn set(&self, data: &[T], update: bool) -> Result<()> {
        self.set_inner(data, None, update)
    }

    /// Like [`Self::set`], replacing the header as well.
    ///
    /// # Errors
    ///
    /// Returns an error when the state has no header type or the header is
    /// larger than a value may be.
    pub fn set_with_header(&self, data: &[T], header: &H, update: bool) -> Result<()> {
        self.set_inner(data, Some(&serialize_bytes(header)?), update)
    }

    fn set_inner(&self, data: &[T], header: Option<&Bytes>, update: bool) -> Result<()> {
        self.inner
            .set(with_header(data, header), update)
            .map_err(ServerError::new)
    }

    /// Appends elements and optionally requests a client repaint.
    pub fn add(&self, data: &[T], update: bool) -> Result<()> {
        self.add_inner(data, None, update)
    }

    /// Like [`Self::add`], replacing the header as well.
    pub fn add_with_header(&self, data: &[T], header: &H, update: bool) -> Result<()> {
        self.add_inner(data, Some(&serialize_bytes(header)?), update)
    }

    fn add_inner(&self, data: &[T], header: Option<&Bytes>, update: bool) -> Result<()> {
        self.inner
            .add(with_header(data, header), update)
            .map_err(ServerError::new)
    }

    /// Replaces elements starting at `index`.
    ///
    /// # Errors
    ///
    /// Returns an error when the replaced range exceeds the current buffer or
    /// the update cannot be serialized or queued.
    pub fn replace(&self, data: &[T], index: usize, update: bool) -> Result<()> {
        self.replace_inner(data, index, None, update)
    }

    /// Like [`Self::replace`], replacing the header as well.
    pub fn replace_with_header(
        &self,
        data: &[T],
        index: usize,
        header: &H,
        update: bool,
    ) -> Result<()> {
        self.replace_inner(data, index, Some(&serialize_bytes(header)?), update)
    }

    fn replace_inner(
        &self,
        data: &[T],
        index: usize,
        header: Option<&Bytes>,
        update: bool,
    ) -> Result<()> {
        self.inner
            .replace(with_header(data, header), index, update)
            .map_err(ServerError::new)
    }

    /// Removes `count` elements starting at `index`.
    ///
    /// # Errors
    ///
    /// Returns an error when the removed range exceeds the current buffer or
    /// the update cannot be queued.
    pub fn remove(&self, index: usize, count: usize, update: bool) -> Result<()> {
        self.inner
            .remove(index, count, None, update)
            .map_err(ServerError::new)
    }

    /// Like [`Self::remove`], replacing the header as well.
    pub fn remove_with_header(
        &self,
        index: usize,
        count: usize,
        header: &H,
        update: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .remove(index, count, Some(&header), update)
            .map_err(ServerError::new)
    }

    /// Removes all elements and optionally requests a client repaint. The
    /// header stays.
    pub fn clear(&self, update: bool) -> Result<()> {
        self.inner.clear(None, update).map_err(ServerError::new)
    }

    /// Like [`Self::clear`], replacing the header as well.
    pub fn clear_with_header(&self, header: &H, update: bool) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .clear(Some(&header), update)
            .map_err(ServerError::new)
    }

    /// Replaces the header alone.
    ///
    /// Unlike buffer changes, the client does not acknowledge a header, so
    /// this does not wait for the client itself: headers set faster than the
    /// client reads them queue up, as values do. It is queued after a buffer
    /// change that another thread is still sending, though, and waits as long
    /// as that change waits for the client.
    pub fn set_header(&self, header: &H, update: bool) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .set_header(&header, update)
            .map_err(ServerError::new)
    }
}

/// One-shot numeric buffer sent by the server and consumed by the client,
/// with an optional typed header `H`.
pub struct DataTake<T, H = ()> {
    inner: Arc<CoreDataTake>,
    _type: PhantomData<fn() -> (T, H)>,
}

impl<T, H> Clone for DataTake<T, H> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _type: PhantomData,
        }
    }
}

impl<T, H> DataTake<T, H>
where
    T: DataElement,
    H: HeaderValue,
{
    /// Registers a one-shot numeric buffer under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data_take::<T>(name.into(), header_type::<H>())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    /// Sends `data` to the client; the client takes it with `H::default()`.
    ///
    /// `blocking` waits for the client to consume the previous transfer before
    /// the next send. `cache` retains the last transfer for a newly connecting
    /// client, and `update` requests a repaint.
    pub fn set(&self, data: &[T], blocking: bool, update: bool, cache: bool) -> Result<()> {
        self.inner
            .set(data_slice(data), blocking, update, cache)
            .map_err(ServerError::new)
    }

    /// Like [`Self::set`], sending `header` with the buffer.
    pub fn set_with_header(
        &self,
        data: &[T],
        header: &H,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .set(with_header(data, Some(&header)), blocking, update, cache)
            .map_err(ServerError::new)
    }
}

/// Server-side numeric buffers indexed by `u32` keys, each with an optional
/// typed header `H`.
pub struct DataMulti<T, H = ()> {
    inner: Arc<CoreDataMulti>,
    _type: PhantomData<fn() -> (T, H)>,
}

impl<T, H> Clone for DataMulti<T, H> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _type: PhantomData,
        }
    }
}

impl<T, H> DataMulti<T, H>
where
    T: DataElement,
    H: HeaderValue,
{
    /// Registers an empty keyed buffer collection under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data_multi::<T>(name.into(), header_type::<H>())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    /// Returns the buffer at `index`, or `None` if it is absent.
    pub fn get(&self, index: u32) -> Result<Option<Vec<T>>> {
        self.inner.get(index, |data| match data {
            Some(data) => data_from_bytes(data).map(Some),
            None => Ok(None),
        })
    }

    /// Returns the header at `index`, or `None` if the key is absent or no
    /// header was set for it.
    pub fn header(&self, index: u32) -> Result<Option<H>> {
        self.inner
            .header(index, |header| decode_header(header.flatten()))
    }

    /// Replaces the buffer at `index`. A new key gets the default header.
    pub fn set(&self, index: u32, data: &[T], update: bool) -> Result<()> {
        self.set_inner(index, data, None, update)
    }

    /// Like [`Self::set`], replacing the header as well.
    pub fn set_with_header(&self, index: u32, data: &[T], header: &H, update: bool) -> Result<()> {
        self.set_inner(index, data, Some(&serialize_bytes(header)?), update)
    }

    fn set_inner(
        &self,
        index: u32,
        data: &[T],
        header: Option<&Bytes>,
        update: bool,
    ) -> Result<()> {
        self.inner
            .set(index, with_header(data, header), update)
            .map_err(ServerError::new)
    }

    /// Appends elements to the buffer at `index`.
    pub fn add(&self, index: u32, data: &[T], update: bool) -> Result<()> {
        self.add_inner(index, data, None, update)
    }

    /// Like [`Self::add`], replacing the header as well.
    pub fn add_with_header(&self, index: u32, data: &[T], header: &H, update: bool) -> Result<()> {
        self.add_inner(index, data, Some(&serialize_bytes(header)?), update)
    }

    fn add_inner(
        &self,
        index: u32,
        data: &[T],
        header: Option<&Bytes>,
        update: bool,
    ) -> Result<()> {
        self.inner
            .add(index, with_header(data, header), update)
            .map_err(ServerError::new)
    }

    /// Replaces elements starting at `data_index` in the buffer at `index`.
    pub fn replace(&self, index: u32, data_index: usize, data: &[T], update: bool) -> Result<()> {
        self.replace_inner(index, data_index, data, None, update)
    }

    /// Like [`Self::replace`], replacing the header as well.
    pub fn replace_with_header(
        &self,
        index: u32,
        data_index: usize,
        data: &[T],
        header: &H,
        update: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.replace_inner(index, data_index, data, Some(&header), update)
    }

    fn replace_inner(
        &self,
        index: u32,
        data_index: usize,
        data: &[T],
        header: Option<&Bytes>,
        update: bool,
    ) -> Result<()> {
        self.inner
            .replace(index, data_index, with_header(data, header), update)
            .map_err(ServerError::new)
    }

    /// Removes `count` elements from `data_index` in the selected buffer.
    pub fn remove(&self, index: u32, data_index: usize, count: usize, update: bool) -> Result<()> {
        self.inner
            .remove(index, data_index, count, None, update)
            .map_err(ServerError::new)
    }

    /// Like [`Self::remove`], replacing the header as well.
    pub fn remove_with_header(
        &self,
        index: u32,
        data_index: usize,
        count: usize,
        header: &H,
        update: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .remove(index, data_index, count, Some(&header), update)
            .map_err(ServerError::new)
    }

    /// Clears the buffer at `index` without removing its key or its header.
    pub fn clear(&self, index: u32, update: bool) -> Result<()> {
        self.inner
            .clear(index, None, update)
            .map_err(ServerError::new)
    }

    /// Like [`Self::clear`], replacing the header as well.
    pub fn clear_with_header(&self, index: u32, header: &H, update: bool) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .clear(index, Some(&header), update)
            .map_err(ServerError::new)
    }

    /// Replaces the header at `index` alone. Like [`Data::set_header`], it
    /// does not wait for the client itself, but it is queued after a buffer
    /// change to any key that another thread is still sending.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` does not exist, the state has no header
    /// type or the header is too large.
    pub fn set_header(&self, index: u32, header: &H, update: bool) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .set_header(index, &header, update)
            .map_err(ServerError::new)
    }

    /// Removes the buffer, key and header at `index`.
    pub fn remove_index(&self, index: u32, update: bool) -> Result<()> {
        self.inner
            .remove_index(index, update)
            .map_err(ServerError::new)
    }

    /// Removes every keyed buffer and header.
    pub fn reset(&self, update: bool) -> Result<()> {
        self.inner.reset(update).map_err(ServerError::new)
    }
}

/// Keyed one-shot numeric buffers sent from the server to the client, with
/// an optional typed header `H`.
pub struct DataMultiTake<T, H = ()> {
    inner: Arc<CoreDataMultiTake>,
    _type: PhantomData<fn() -> (T, H)>,
}

impl<T, H> Clone for DataMultiTake<T, H> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _type: PhantomData,
        }
    }
}

impl<T, H> DataMultiTake<T, H>
where
    T: DataElement,
    H: HeaderValue,
{
    /// Registers an empty keyed one-shot collection under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data_multi_take::<T>(name.into(), header_type::<H>())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    /// Sends a buffer at `index` with blocking, repaint, and caching controls;
    /// the client takes it with `H::default()`.
    ///
    /// A reconnected client holds only the cached keys; values it had not
    /// taken before the disconnect are dropped.
    pub fn set(
        &self,
        index: u32,
        data: &[T],
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<()> {
        self.inner
            .set(index, data_slice(data), blocking, update, cache)
            .map_err(ServerError::new)
    }

    /// Like [`Self::set`], sending `header` with the buffer.
    pub fn set_with_header(
        &self,
        index: u32,
        data: &[T],
        header: &H,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .set(
                index,
                with_header(data, Some(&header)),
                blocking,
                update,
                cache,
            )
            .map_err(ServerError::new)
    }

    /// Removes the cached or pending transfer at `index`.
    pub fn remove_index(&self, index: u32, update: bool) -> Result<()> {
        self.inner
            .remove_index(index, update)
            .map_err(ServerError::new)
    }

    /// Removes every cached or pending keyed transfer.
    pub fn reset(&self, update: bool) -> Result<()> {
        self.inner.reset(update).map_err(ServerError::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_round_trips_without_a_client() {
        let server = StateServer::new().unwrap();
        let data = Data::<u16>::new(&server, "root.data").unwrap();

        data.set(&[1, 2, 3], false).unwrap();
        data.add(&[4, 5], false).unwrap();
        data.replace(&[8, 9], 1, false).unwrap();
        assert_eq!(data.get().unwrap(), vec![1, 8, 9, 4, 5]);
        assert_eq!(data.read(|values| values.iter().sum::<u16>()).unwrap(), 27);
        assert_eq!(data.read_bytes(<[u8]>::len), 10);
        data.remove(2, 2, false).unwrap();
        assert_eq!(data.get().unwrap(), vec![1, 8, 5]);
        data.clear(false).unwrap();
        assert!(data.get().unwrap().is_empty());
    }

    #[test]
    fn multi_data_round_trips_without_a_client() {
        let server = StateServer::new().unwrap();
        let data = DataMulti::<f32>::new(&server, "root.data").unwrap();

        data.set(3, &[1.0, 2.0], false).unwrap();
        data.add(3, &[3.0], false).unwrap();
        data.replace(3, 1, &[4.0], false).unwrap();
        assert_eq!(data.get(3).unwrap(), Some(vec![1.0, 4.0, 3.0]));
        data.remove(3, 0, 1, false).unwrap();
        assert_eq!(data.get(3).unwrap(), Some(vec![4.0, 3.0]));
        data.remove_index(3, false).unwrap();
        assert_eq!(data.get(3).unwrap(), None);
    }

    #[test]
    fn invalid_data_ranges_are_rejected_without_changes() {
        let server = StateServer::new().unwrap();
        let data = Data::<u16>::new(&server, "root.data").unwrap();
        data.set(&[1, 2, 3], false).unwrap();

        assert!(data.replace(&[9, 9], 2, false).is_err());
        assert!(data.remove(2, 2, false).is_err());
        assert!(data.remove(0, 0, false).is_err());
        assert_eq!(data.get().unwrap(), [1, 2, 3]);

        // Empty edits at the end are valid no-ops.
        data.replace(&[], 3, false).unwrap();
        data.add(&[], false).unwrap();
        assert_eq!(data.get().unwrap(), [1, 2, 3]);
    }

    /// Range checks must not overflow: a huge index is an ordinary error.
    #[test]
    fn overflowing_data_ranges_are_rejected_without_changes() {
        let server = StateServer::new().unwrap();
        let data = Data::<u16>::new(&server, "root.data").unwrap();
        let multi = DataMulti::<u16>::new(&server, "root.multi").unwrap();
        data.set(&[1, 2, 3], false).unwrap();
        multi.set(4, &[1, 2, 3], false).unwrap();

        type Attempt<'a> = (&'a str, Box<dyn Fn() -> Result<()> + 'a>);
        let attempts: [Attempt<'_>; 4] = [
            (
                "replace",
                Box::new(|| data.replace(&[9], usize::MAX, false)),
            ),
            ("remove", Box::new(|| data.remove(usize::MAX, 1, false))),
            (
                "multi replace",
                Box::new(|| multi.replace(4, usize::MAX, &[9], false)),
            ),
            (
                "multi remove",
                Box::new(|| multi.remove(4, usize::MAX, 1, false)),
            ),
        ];
        for (name, attempt) in attempts {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(attempt));
            assert!(
                matches!(result, Ok(Err(_))),
                "{name} with a huge index did not return an error"
            );
        }
        assert_eq!(data.get().unwrap(), [1, 2, 3]);
        assert_eq!(multi.get(4).unwrap(), Some(vec![1, 2, 3]));
    }

    #[test]
    fn multi_data_edits_require_an_existing_key_and_range() {
        let server = StateServer::new().unwrap();
        let data = DataMulti::<u8>::new(&server, "root.data").unwrap();
        data.set(1, &[1, 2, 3], false).unwrap();

        assert!(data.add(2, &[4], false).is_err());
        assert!(data.replace(2, 0, &[4], false).is_err());
        assert!(data.remove(2, 0, 1, false).is_err());
        assert!(data.clear(2, false).is_err());
        assert_eq!(data.get(2).unwrap(), None);

        assert!(data.replace(1, 2, &[8, 9], false).is_err());
        assert!(data.remove(1, 1, 3, false).is_err());
        assert!(data.remove(1, 0, 0, false).is_err());
        assert_eq!(data.get(1).unwrap(), Some(vec![1, 2, 3]));

        data.clear(1, false).unwrap();
        assert_eq!(data.get(1).unwrap(), Some(vec![]));
        // Removing or resetting missing keys is harmless.
        data.remove_index(7, false).unwrap();
        data.reset(false).unwrap();
        data.reset(false).unwrap();
        assert_eq!(data.get(1).unwrap(), None);
    }

    #[test]
    fn headers_are_stored_with_the_data() {
        type Meta = (u32, String);
        let server = StateServer::new().unwrap();
        let data = Data::<u16, Meta>::new(&server, "root.data").unwrap();
        let multi = DataMulti::<u16, Meta>::new(&server, "root.multi").unwrap();
        let take = DataTake::<u16, Meta>::new(&server, "root.take").unwrap();
        let plain = Data::<u16>::new(&server, "root.plain").unwrap();

        assert_eq!(data.header().unwrap(), None);
        data.set_with_header(&[1, 2], &(1, "a".into()), false)
            .unwrap();
        data.add(&[3], false).unwrap();
        assert_eq!(data.header().unwrap(), Some((1, "a".into())));
        data.clear_with_header(&(2, "b".into()), false).unwrap();
        data.set_header(&(3, "c".into()), false).unwrap();
        assert_eq!(data.get().unwrap(), Vec::<u16>::new());
        assert_eq!(data.header().unwrap(), Some((3, "c".into())));

        multi.set(1, &[1], false).unwrap();
        assert_eq!(multi.header(1).unwrap(), None);
        multi
            .replace_with_header(1, 0, &[5], &(4, "d".into()), false)
            .unwrap();
        assert_eq!(multi.header(1).unwrap(), Some((4, "d".into())));
        assert!(multi.set_header(2, &(5, "e".into()), false).is_err());
        assert_eq!(multi.header(2).unwrap(), None);

        take.set_with_header(&[1], &(6, "f".into()), false, false, true)
            .unwrap();

        let error = plain.set_with_header(&[1], &(), false).unwrap_err();
        assert!(error.to_string().contains("without a header"), "{error}");
        assert_eq!(plain.header().unwrap(), None);
    }
}
