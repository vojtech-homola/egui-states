use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use serde::Deserialize;

use egui_states_core::collections::MapHeader;
use egui_states_core::serialization::deserialize;
use egui_states_core::types::GetType;

pub(crate) trait UpdateMap: Sync + Send {
    fn update_map(&self, header: MapHeader, data: &[u8]) -> Result<(), String>;
}

pub struct ValueMap<K, V> {
    _id: u64,
    dict: RwLock<HashMap<K, V>>,
}

impl<K, V> ValueMap<K, V>
where
    K: GetType + Clone + Hash + Eq,
    V: GetType + Clone,
{
    pub(crate) fn new(id: u64) -> Arc<Self> {
        Arc::new(Self {
            _id: id,
            dict: RwLock::new(HashMap::new()),
        })
    }

    #[inline]
    pub fn get(&self) -> HashMap<K, V> {
        self.dict.read().clone()
    }

    #[inline]
    pub fn get_item(&self, key: &K) -> Option<V> {
        self.dict.read().get(key).cloned()
    }

    pub fn read<R>(&self, mut f: impl FnMut(&HashMap<K, V>) -> R) -> R {
        let d = self.dict.read();
        f(&*d)
    }

    pub fn read_item<R>(&self, key: &K, mut f: impl FnMut(Option<&V>) -> R) -> R {
        let d = self.dict.read();
        let v = d.get(key);
        f(v)
    }
}

impl<K, V> UpdateMap for ValueMap<K, V>
where
    K: for<'a> Deserialize<'a> + Eq + Hash + Send + Sync,
    V: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_map(&self, header: MapHeader, data: &[u8]) -> Result<(), String> {
        match header {
            MapHeader::All => {
                let map = deserialize::<HashMap<K, V>>(data)
                    .map_err(|e| format!("Error deserializing dict for id {}: {}", self._id, e))?;
                *self.dict.write() = map;
            }
            MapHeader::Set => {
                let (key, value): (K, V) = deserialize(data).map_err(|e| {
                    format!("Error deserializing dict item for id {}: {}", self._id, e)
                })?;
                self.dict.write().insert(key, value);
            }
            MapHeader::Remove => {
                let key: K = deserialize(data).map_err(|e| {
                    format!("Error deserializing dict key for id {}: {}", self._id, e)
                })?;
                self.dict.write().remove(&key);
            }
        }
        Ok(())
    }
}
