use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::Typed;
use crate::server_core::map_core::ValueMap as CoreMap;
use crate::server_core::vec_core::ValueList as CoreVec;

use super::state_server::{StateServer, deserialize_bytes, serialize_bytes};
use super::{Result, ServerError};

#[derive(Clone)]
/// Server-controlled vector mirrored on the client.
pub struct VecState<T> {
    inner: Arc<CoreVec>,
    _type: PhantomData<T>,
}

impl<T> VecState<T>
where
    T: Typed,
{
    /// Registers an empty vector under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_vec::<T>(name.into())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }
}

impl<T> VecState<T>
where
    T: Serialize,
{
    /// Replaces the complete vector and optionally requests a client repaint.
    pub fn set(&self, value: Vec<T>, update: bool) -> Result<()> {
        let data = value
            .iter()
            .map(serialize_bytes)
            .collect::<Result<Vec<_>>>()?;
        self.inner.set(data, update).map_err(ServerError::new)
    }

    /// Replaces the item at `index` and optionally requests a repaint.
    pub fn set_item(&self, index: usize, value: T, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner
            .set_item_py(index, data, update)
            .map_err(ServerError::new)
    }

    /// Appends an item and optionally requests a repaint.
    pub fn add_item(&self, value: T, update: bool) -> Result<()> {
        let data = serialize_bytes(&value)?;
        self.inner
            .append_item(data, update)
            .map_err(ServerError::new)
    }
}

impl<T> VecState<T>
where
    T: for<'a> Deserialize<'a>,
{
    /// Returns the complete vector.
    pub fn get(&self) -> Result<Vec<T>> {
        self.inner
            .get()
            .iter()
            .map(|data| deserialize_bytes(data))
            .collect()
    }

    /// Returns the item at `index`.
    pub fn get_item(&self, index: usize) -> Result<T> {
        let data = self.inner.get_item(index).map_err(ServerError::new)?;
        deserialize_bytes(&data)
    }

    /// Removes and returns the item at `index`.
    pub fn remove_item(&self, index: usize, update: bool) -> Result<T> {
        let data = self
            .inner
            .remove_item(index, update)
            .map_err(ServerError::new)?;
        deserialize_bytes(&data)
    }
}

impl<T> VecState<T> {
    /// Returns the number of items.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the vector contains no items.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }
}

#[derive(Clone)]
/// Server-controlled hash map mirrored on the client.
pub struct MapState<K, V> {
    inner: Arc<CoreMap>,
    _type: PhantomData<(K, V)>,
}

impl<K, V> MapState<K, V>
where
    K: Typed,
    V: Typed,
{
    /// Registers an empty map under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_map::<K, V>(name.into())?;
        Ok(Self {
            inner,
            _type: PhantomData,
        })
    }
}

impl<K, V> MapState<K, V>
where
    K: Serialize,
    V: Serialize,
{
    /// Replaces the complete map and optionally requests a client repaint.
    pub fn set(&self, value: HashMap<K, V>, update: bool) -> Result<()> {
        let mut map = HashMap::with_capacity(value.len());
        for (key, value) in value {
            map.insert(serialize_bytes(&key)?, serialize_bytes(&value)?);
        }
        self.inner.set(map, update).map_err(ServerError::new)
    }

    /// Inserts or replaces one entry and optionally requests a repaint.
    pub fn set_item(&self, key: K, value: V, update: bool) -> Result<()> {
        self.inner
            .set_item(serialize_bytes(&key)?, serialize_bytes(&value)?, update)
            .map_err(ServerError::new)
    }
}

impl<K, V> MapState<K, V>
where
    K: for<'a> Deserialize<'a> + Eq + Hash,
    V: for<'a> Deserialize<'a>,
{
    /// Returns the complete map.
    pub fn get(&self) -> Result<HashMap<K, V>> {
        let raw = self.inner.get();
        let mut map = HashMap::with_capacity(raw.len());
        for (key, value) in raw {
            map.insert(deserialize_bytes(&key)?, deserialize_bytes(&value)?);
        }
        Ok(map)
    }
}

impl<K, V> MapState<K, V>
where
    K: Serialize,
    V: for<'a> Deserialize<'a>,
{
    /// Returns the value at `key`, or `None` if the key is absent.
    pub fn get_item(&self, key: &K) -> Result<Option<V>> {
        let key_data = serialize_bytes(key)?;
        match self.inner.get_item(&key_data) {
            Some(value) => Ok(Some(deserialize_bytes(&value)?)),
            None => Ok(None),
        }
    }

    /// Removes and returns the value at `key`, if present.
    pub fn remove_item(&self, key: &K, update: bool) -> Result<Option<V>> {
        let key_data = serialize_bytes(key)?;
        match self
            .inner
            .remove_item(&key_data, update)
            .map_err(|_| ServerError::new("failed to remove map item"))?
        {
            Some(value) => Ok(Some(deserialize_bytes(&value)?)),
            None => Ok(None),
        }
    }
}

impl<K, V> MapState<K, V> {
    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the map contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collections_round_trip_without_a_client() {
        let server = StateServer::new(0).unwrap();
        let values = VecState::<i32>::new(&server, "root.values").unwrap();
        let map = MapState::<u16, String>::new(&server, "root.map").unwrap();

        values.set(vec![1, 2], false).unwrap();
        values.add_item(3, false).unwrap();
        values.set_item(1, 4, false).unwrap();
        assert_eq!(values.get().unwrap(), vec![1, 4, 3]);
        assert_eq!(values.remove_item(0, false).unwrap(), 1);
        assert_eq!(values.get().unwrap(), vec![4, 3]);

        map.set(HashMap::from([(1, String::from("one"))]), false)
            .unwrap();
        map.set_item(2, String::from("two"), false).unwrap();
        assert_eq!(map.get_item(&2).unwrap().as_deref(), Some("two"));
        assert_eq!(map.remove_item(&1, false).unwrap().as_deref(), Some("one"));
        assert_eq!(map.len(), 1);
    }
}
