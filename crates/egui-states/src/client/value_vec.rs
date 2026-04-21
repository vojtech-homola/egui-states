use parking_lot::RwLock;
use std::sync::Arc;

use serde::Deserialize;

use crate::collections::VecHeader;
use crate::serialization::{Deserializer, deserialize};
use crate::transport::Transportable;

pub(crate) trait UpdateList: Sync + Send {
    fn update_list(&self, type_id: u32, header: VecHeader, data: &[u8]) -> Result<(), String>;
}

pub struct ValueVec<T> {
    name: String,
    type_id: u32,
    list: Arc<RwLock<Vec<T>>>,
}

impl<T: Transportable + Clone> ValueVec<T> {
    pub(crate) fn new(name: String, type_id: u32) -> Self {
        Self {
            name,
            type_id,
            list: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn get(&self) -> Vec<T> {
        self.list.read().clone()
    }

    pub fn get_item(&self, idx: usize) -> Option<T> {
        self.list.read().get(idx).cloned()
    }

    pub fn read<R>(&self, mut f: impl FnMut(&Vec<T>) -> R) -> R {
        let l = self.list.read();
        f(&*l)
    }

    pub fn read_item<R>(&self, idx: usize, mut f: impl FnMut(Option<&T>) -> R) -> R {
        let l = self.list.read();
        f(l.get(idx))
    }
}

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateList for ValueVec<T> {
    fn update_list(&self, type_id: u32, header: VecHeader, data: &[u8]) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for list {}", self.name));
        }

        match header {
            VecHeader::All(size) => {
                let mut deserializer = Deserializer::new(data);

                let mut list = self.list.write();
                list.clear();
                list.reserve(size as usize);

                for _ in 0..size {
                    let item: T = deserializer.get().map_err(|e| {
                        format!("Error deserializing list item for {}: {}", self.name, e)
                    })?;
                    list.push(item);
                }
            }
            VecHeader::Set(idx) => {
                let value: T = deserialize(data).map_err(|e| {
                    format!("Error deserializing list item for {}: {}", self.name, e)
                })?;
                let mut list = self.list.write();
                let idx = idx as usize;
                if idx < list.len() {
                    list[idx] = value;
                }
            }
            VecHeader::Add => {
                let value: T = deserialize(data).map_err(|e| {
                    format!("Error deserializing list item for {}: {}", self.name, e)
                })?;
                self.list.write().push(value);
            }
            VecHeader::Remove(idx) => {
                let mut list = self.list.write();
                let idx = idx as usize;
                if idx < list.len() {
                    list.remove(idx);
                }
            }
        }
        Ok(())
    }
}

impl<T> Clone for ValueVec<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            type_id: self.type_id,
            list: self.list.clone(),
        }
    }
}
