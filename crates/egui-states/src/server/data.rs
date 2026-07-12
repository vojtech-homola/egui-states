use std::marker::PhantomData;
use std::sync::Arc;

use crate::data_transport::DataType;
use crate::server_core::data_core::{Data as CoreData, DataHolder, DataMulti as CoreDataMulti};
use crate::server_core::data_take_core::{
    DataMultiTake as CoreDataMultiTake, DataTake as CoreDataTake,
};

use super::state_server::StateServer;
use super::{Result, ServerError};

/// A numeric element that can be transported by a [`Data`] state.
///
/// # Safety
///
/// `TYPE_ID` must identify a protocol data type with exactly the same size as
/// `Self`. Every bit pattern received for that protocol type must also be a
/// valid value of `Self`, and `Self` must not contain padding bytes.
pub unsafe trait DataElement: Copy + Send + Sync + 'static {
    const TYPE_ID: u8;
}

macro_rules! impl_data_element {
    ($(($ty:ty, $id:expr)),* $(,)?) => {
        $(
            unsafe impl DataElement for $ty {
                const TYPE_ID: u8 = $id;
            }
        )*
    };
}

impl_data_element! {
    (u8, 0),
    (u16, 1),
    (u32, 2),
    (u64, 3),
    (i8, 4),
    (i16, 5),
    (i32, 6),
    (i64, 7),
    (f32, 8),
    (f64, 9),
}

fn data_type<T: DataElement>() -> DataType {
    DataType::from_id(T::TYPE_ID).expect("DataElement type id must be valid")
}

fn data_holder<T: DataElement>(data: &[T]) -> DataHolder {
    DataHolder {
        data: data.as_ptr() as *const u8,
        count: data.len(),
        data_size: std::mem::size_of_val(data),
        data_type: data_type::<T>(),
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

#[derive(Clone)]
pub struct Data<T> {
    inner: Arc<CoreData>,
    _type: PhantomData<T>,
}

impl<T> Data<T>
where
    T: DataElement,
{
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data::<T>(name.into())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    pub fn get(&self) -> Result<Vec<T>> {
        self.inner.get(data_from_bytes)
    }

    pub fn read<R>(&self, f: impl FnOnce(&[T]) -> R) -> Result<R> {
        let data = self.get()?;
        Ok(f(&data))
    }

    pub fn read_bytes<R>(&self, f: impl Fn(&[u8]) -> R) -> R {
        self.inner.get(f)
    }

    pub fn set(&self, data: &[T], update: bool) -> Result<()> {
        self.inner
            .set(data_holder(data), update)
            .map_err(ServerError::new)
    }

    pub fn add(&self, data: &[T], update: bool) -> Result<()> {
        self.inner
            .add(data_holder(data), update)
            .map_err(ServerError::new)
    }

    pub fn replace(&self, data: &[T], index: usize, update: bool) -> Result<()> {
        self.inner
            .replace(data_holder(data), index, update)
            .map_err(ServerError::new)
    }

    pub fn remove(&self, index: usize, count: usize, update: bool) -> Result<()> {
        self.inner
            .remove(index, count, update)
            .map_err(ServerError::new)
    }

    pub fn clear(&self, update: bool) -> Result<()> {
        self.inner.clear(update).map_err(ServerError::new)
    }
}

#[derive(Clone)]
pub struct DataTake<T> {
    inner: Arc<CoreDataTake>,
    _type: PhantomData<T>,
}

impl<T> DataTake<T>
where
    T: DataElement,
{
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data_take::<T>(name.into())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    pub fn set(&self, data: &[T], blocking: bool, update: bool, cache: bool) -> Result<()> {
        self.inner
            .set(data_holder(data), blocking, update, cache)
            .map_err(ServerError::new)
    }
}

#[derive(Clone)]
pub struct DataMulti<T> {
    inner: Arc<CoreDataMulti>,
    _type: PhantomData<T>,
}

impl<T> DataMulti<T>
where
    T: DataElement,
{
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data_multi::<T>(name.into())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    pub fn get(&self, index: u32) -> Result<Option<Vec<T>>> {
        self.inner.get(index, |data| match data {
            Some(data) => data_from_bytes(data).map(Some),
            None => Ok(None),
        })
    }

    pub fn set(&self, index: u32, data: &[T], update: bool) -> Result<()> {
        self.inner
            .set(index, data_holder(data), update)
            .map_err(ServerError::new)
    }

    pub fn add(&self, index: u32, data: &[T], update: bool) -> Result<()> {
        self.inner
            .add(index, data_holder(data), update)
            .map_err(ServerError::new)
    }

    pub fn replace(&self, index: u32, data_index: usize, data: &[T], update: bool) -> Result<()> {
        self.inner
            .replace(index, data_index, data_holder(data), update)
            .map_err(ServerError::new)
    }

    pub fn remove(&self, index: u32, data_index: usize, count: usize, update: bool) -> Result<()> {
        self.inner
            .remove(index, data_index, count, update)
            .map_err(ServerError::new)
    }

    pub fn clear(&self, index: u32, update: bool) -> Result<()> {
        self.inner.clear(index, update).map_err(ServerError::new)
    }

    pub fn remove_index(&self, index: u32, update: bool) -> Result<()> {
        self.inner
            .remove_index(index, update)
            .map_err(ServerError::new)
    }

    pub fn reset(&self, update: bool) -> Result<()> {
        self.inner.reset(update).map_err(ServerError::new)
    }
}

#[derive(Clone)]
pub struct DataMultiTake<T> {
    inner: Arc<CoreDataMultiTake>,
    _type: PhantomData<T>,
}

impl<T> DataMultiTake<T>
where
    T: DataElement,
{
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_data_multi_take::<T>(name.into())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }

    pub fn set(
        &self,
        index: u32,
        data: &[T],
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<()> {
        self.inner
            .set(index, data_holder(data), blocking, update, cache)
            .map_err(ServerError::new)
    }

    pub fn remove_index(&self, index: u32, update: bool) -> Result<()> {
        self.inner
            .remove_index(index, update)
            .map_err(ServerError::new)
    }

    pub fn reset(&self, update: bool) -> Result<()> {
        self.inner.reset(update).map_err(ServerError::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_round_trips_without_a_client() {
        let server = StateServer::new(0).unwrap();
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
        let server = StateServer::new(0).unwrap();
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
}
