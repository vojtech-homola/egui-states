use std::sync::{Arc, RwLock};

use egui_pysync_transport::collections::ItemWriteRead;
use egui_pysync_transport::list::ListMessage;

pub(crate) trait ListUpdate: Sync + Send {
    fn update_list(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String>;
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
    fn update_list(&self, head: &[u8], data: Option<Vec<u8>>) -> Result<(), String> {
        let message = ListMessage::read_message(head, data)?;
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
