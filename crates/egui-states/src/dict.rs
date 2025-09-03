use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use egui_states_core::serialization::deserialize;

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
        self.dict.read().unwrap().clone()
    }

    #[inline]
    pub fn get_item(&self, key: &K) -> Option<V> {
        self.dict.read().unwrap().get(key).cloned()
    }

    pub fn process<R>(&self, op: impl Fn(&HashMap<K, V>) -> R) -> R {
        let d = self.dict.read().unwrap();
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
                *self.dict.write().unwrap() = dict;
            }
            DictMessage::Set(key, value) => {
                self.dict.write().unwrap().insert(key, value);
            }
            DictMessage::Remove(key) => {
                self.dict.write().unwrap().remove(&key);
            }
        }
        Ok(update)
    }
}
