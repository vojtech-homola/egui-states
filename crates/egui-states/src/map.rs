use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use serde::Deserialize;

use egui_states_core::collections::MapHeader;
use egui_states_core::serialization::deserialize;
use egui_states_core::values::GetType;

pub(crate) trait UpdateMap: Sync + Send {
    fn update_map(
        &self,
        type_ids: (u64, u64),
        header: MapHeader,
        data: &[u8],
    ) -> Result<(), String>;
}

pub struct ValueMap<K, V> {
    _id: u64,
    type_ids: (u64, u64),
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
            type_ids: (K::get_type().get_hash(), V::get_type().get_hash()),
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

    pub fn process<R>(&self, op: impl Fn(&HashMap<K, V>) -> R) -> R {
        let d = self.dict.read();
        op(&*d)
    }
}

impl<K, V> UpdateMap for ValueMap<K, V>
where
    K: for<'a> Deserialize<'a> + Eq + Hash + Send + Sync,
    V: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_map(
        &self,
        type_ids: (u64, u64),
        header: MapHeader,
        data: &[u8],
    ) -> Result<(), String> {
        if self.type_ids != type_ids {
            return Err(format!(
                "Type mismatch for dict id: {} expected: {:?} got: {:?}",
                self._id, self.type_ids, type_ids
            ));
        }

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
