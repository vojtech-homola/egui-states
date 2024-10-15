use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, RwLock};

use egui_pytransport::collections::ItemWriteRead;
use egui_pytransport::dict::DictMessage;

pub(crate) trait DictUpdate: Sync + Send {
    fn update_dict(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String>;
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

impl<K, V> DictUpdate for ValueDict<K, V>
where
    K: ItemWriteRead + Eq + Hash,
    V: ItemWriteRead,
{
    fn update_dict(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String> {
        let message: DictMessage<K, V> = DictMessage::read_message(head, data)?;
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
        Ok(())
    }
}
