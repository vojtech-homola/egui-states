use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use serde::Deserialize;

use egui_states_core_old::serialization::deserialize;

use crate::UpdateValue;

#[derive(Deserialize)]
enum DictMessage<K, V>
where
    K: Eq + Hash,
{
    All(HashMap<K, V>),
    Set(K, V),
    Remove(K),
}

pub struct ValueDict<K, V> {
    _id: u32,
    dict: RwLock<HashMap<K, V>>,
}

impl<K, V> ValueDict<K, V>
where
    K: Clone + Hash + Eq,
    V: Clone,
{
    pub(crate) fn new(id: u32) -> Arc<Self> {
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

    pub fn process<R>(&self, op: impl Fn(&HashMap<K, V>) -> R) -> R {
        let d = self.dict.read();
        op(&*d)
    }
}

impl<K, V> UpdateValue for ValueDict<K, V>
where
    K: for<'a> Deserialize<'a> + Eq + Hash + Send + Sync,
    V: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_value(&self, data: &[u8]) -> Result<bool, String> {
        let (update, message) = deserialize(data).map_err(|e| e.to_string())?;
        match message {
            DictMessage::All(dict) => {
                *self.dict.write() = dict;
            }
            DictMessage::Set(key, value) => {
                self.dict.write().insert(key, value);
            }
            DictMessage::Remove(key) => {
                self.dict.write().remove(&key);
            }
        }
        Ok(update)
    }
}
