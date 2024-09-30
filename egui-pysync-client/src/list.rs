use std::io::Read;
use std::net::TcpStream;
use std::sync::{Arc, RwLock};

use egui_pysync_common::collections::ItemWriteRead;
use egui_pysync_common::transport::{self, ListMessage, ParseError};

pub(crate) fn read_message<T: ItemWriteRead>(
    head: &[u8],
    stream: &mut TcpStream,
) -> Result<ListMessage<T>, ParseError> {
    let subtype = head[0];
    match subtype {
        transport::LIST_ALL => {
            let count = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;
            let size = u64::from_le_bytes(head[9..17].try_into().unwrap()) as usize;

            let list = if size > 0 {
                let mut data = vec![0; size];
                stream
                    .read_exact(&mut data)
                    .map_err(|e| ParseError::Connection(e))?;

                let mut list = Vec::new();
                let item_size = T::size();
                for i in 0..count {
                    let value = T::read(&data[i * item_size..]);
                    list.push(value);
                }
                list
            } else {
                Vec::new()
            };

            Ok(ListMessage::All(list))
        }

        transport::LIST_SET => {
            let has_data = head[1] != 0;
            let idx = u64::from_le_bytes(head[2..10].try_into().unwrap()) as usize;

            if has_data {
                let size = u32::from_le_bytes(head[10..14].try_into().unwrap()) as usize;
                let mut data = vec![0; size];
                stream
                    .read_exact(&mut data)
                    .map_err(|e| ParseError::Connection(e))?;
                let value = T::read(&data[0..]);
                return Ok(ListMessage::Set(idx, value));
            }

            let value = T::read(&head[10..]);
            Ok(ListMessage::Set(idx, value))
        }

        transport::LIST_ADD => {
            let has_data = head[1] != 0;

            if has_data {
                let size = u32::from_le_bytes(head[2..6].try_into().unwrap()) as usize;
                let mut data = vec![0; size];
                stream
                    .read_exact(&mut data)
                    .map_err(|e| ParseError::Connection(e))?;
                let value = T::read(&data[0..]);
                return Ok(ListMessage::Add(value));
            }

            let value = T::read(&head[2..]);
            Ok(ListMessage::Add(value))
        }

        transport::LIST_REMOVE => {
            let idx = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;
            Ok(ListMessage::Remove(idx))
        }

        _ => Err(ParseError::Parse(format!(
            "Unknown type of the dict message: {}",
            subtype,
        ))),
    }
}

pub(crate) trait ListUpdate: Sync + Send {
    fn update_list(&self, head: &[u8], stream: &mut TcpStream) -> Result<(), ParseError>;
}

pub struct ValueList<T> {
    _id: u32,
    list: RwLock<Vec<T>>,
}

impl<T: Clone> ValueList<T> {
    pub(crate) fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            _id: id,
            list: RwLock::new(Vec::new()),
        })
    }

    pub fn get(&self) -> Vec<T> {
        self.list.read().unwrap().clone()
    }

    pub fn get_item(&self, idx: usize) -> Option<T> {
        self.list.read().unwrap().get(idx).cloned()
    }
}

impl<T: ItemWriteRead> ListUpdate for ValueList<T> {
    fn update_list(&self, head: &[u8], stream: &mut TcpStream) -> Result<(), ParseError> {
        let message = read_message::<T>(head, stream)?;
        match message {
            ListMessage::All(list) => {
                *self.list.write().unwrap() = list;
            }
            ListMessage::Set(idx, value) => {
                let mut list = self.list.write().unwrap();
                if idx < list.len() {
                    list[idx] = value;
                }
            }
            ListMessage::Add(value) => {
                self.list.write().unwrap().push(value);
            }
            ListMessage::Remove(idx) => {
                let mut list = self.list.write().unwrap();
                if idx < list.len() {
                    list.remove(idx);
                }
            }
        }
        Ok(())
    }
}
