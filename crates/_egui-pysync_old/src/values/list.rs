use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::transport::{MessageData, deserialize};

#[derive(Deserialize)]
enum ListMessage<T> {
    All(Vec<T>),
    Set(usize, T),
    Add(T),
    Remove(usize),
}

pub(crate) trait ListUpdate: Sync + Send {
    fn update_list(&self, data: MessageData) -> Result<(), String>;
}

pub struct ValueList<T> {
    id: u32,
    list: RwLock<Vec<T>>,
}

impl<T: Clone> ValueList<T> {
    pub(crate) fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            id,
            list: RwLock::new(Vec::new()),
        })
    }

    pub fn get(&self) -> Vec<T> {
        self.list.read().unwrap().clone()
    }

    pub fn get_item(&self, idx: usize) -> Option<T> {
        self.list.read().unwrap().get(idx).cloned()
    }

    pub fn process<R>(&self, op: impl Fn(&Vec<T>) -> R) -> R {
        let l = self.list.read().unwrap();
        op(&*l)
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> ListUpdate for ValueList<T> {
    fn update_list(&self, data: MessageData) -> Result<(), String> {
        let message = deserialize(data)
            .map_err(|e| format!("Error deserializing message {} with id {}", e, self.id))?;

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
