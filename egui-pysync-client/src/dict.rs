use std::collections::HashMap;
use std::hash::Hash;
use std::io::Read;
use std::net::TcpStream;
use std::sync::{Arc, RwLock};

use egui_pysync_common::collections::ItemWriteRead;
use egui_pysync_common::transport::{self, DictMessage, ParseError};

pub(crate) fn read_message<K, T>(
    head: &[u8],
    stream: &mut TcpStream,
) -> Result<DictMessage<K, T>, ParseError>
where
    K: ItemWriteRead + Eq + Hash,
    T: ItemWriteRead,
{
    let subtype = head[0];
    match subtype {
        transport::DICT_ALL => {
            let count = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;
            let size = u64::from_le_bytes(head[9..17].try_into().unwrap()) as usize;

            let dict = if size > 0 {
                let mut data = vec![0; size];
                stream
                    .read_exact(&mut data)
                    .map_err(|e| ParseError::Connection(e))?;

                let mut dict = HashMap::new();
                let bouth_size = K::size() + T::size();
                for i in 0..count {
                    let key = K::read(&data[i * bouth_size..]);
                    let value = T::read(&data[i * bouth_size + K::size()..]);
                    dict.insert(key, value);
                }
                dict
            } else {
                HashMap::new()
            };

            Ok(DictMessage::All(dict))
        }

        transport::DICT_SET => {
            let has_data = head[1] != 0;

            if has_data {
                let size = u32::from_le_bytes(head[2..6].try_into().unwrap()) as usize;
                let mut data = vec![0; size];
                stream
                    .read_exact(&mut data)
                    .map_err(|e| ParseError::Connection(e))?;
                let key = K::read(&data[0..]);
                let value = T::read(&data[K::size()..]);
                return Ok(DictMessage::Set(key, value));
            }

            let key = K::read(&head[2..]);
            let value = T::read(&head[2 + K::size()..]);
            Ok(DictMessage::Set(key, value))
        }

        transport::DICT_REMOVE => {
            let has_data = head[1] != 0;

            if has_data {
                let size = u32::from_le_bytes(head[2..6].try_into().unwrap()) as usize;
                let mut data = vec![0; size];
                stream
                    .read_exact(&mut data)
                    .map_err(|e| ParseError::Connection(e))?;
                let key = K::read(&data[0..]);
                return Ok(DictMessage::Remove(key));
            }

            let key = K::read(&head[2..]);
            Ok(DictMessage::Remove(key))
        }

        _ => Err(ParseError::Parse(format!(
            "Unknown type of the dict message: {}",
            subtype,
        ))),
    }
}

pub(crate) trait DictUpdate: Sync + Send {
    fn update_dict(&self, head: &[u8], stream: &mut TcpStream) -> Result<(), ParseError>;
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
}

impl<K, V> DictUpdate for ValueDict<K, V>
where
    K: ItemWriteRead + Eq + Hash,
    V: ItemWriteRead,
{
    fn update_dict(&self, head: &[u8], stream: &mut TcpStream) -> Result<(), ParseError> {
        let message: DictMessage<K, V> = read_message(head, stream)?;
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
