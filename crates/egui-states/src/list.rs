use parking_lot::RwLock;
use std::sync::Arc;

use serde::Deserialize;

use egui_states_core::collections::ListHeader;
use egui_states_core::serialization::deserialize;
use egui_states_core::types::GetType;

pub(crate) trait UpdateList: Sync + Send {
    fn update_list(&self, header: ListHeader, data: &[u8]) -> Result<(), String>;
}

pub struct ValueList<T> {
    id: u64,
    list: RwLock<Vec<T>>,
}

impl<T: GetType + Clone> ValueList<T> {
    pub(crate) fn new(id: u64) -> Arc<Self> {
        Arc::new(Self {
            id,
            list: RwLock::new(Vec::new()),
        })
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

impl<T: for<'a> Deserialize<'a> + Send + Sync> UpdateList for ValueList<T> {
    fn update_list(&self, header: ListHeader, data: &[u8]) -> Result<(), String> {
        match header {
            ListHeader::All => {
                let list: Vec<T> = deserialize(data)
                    .map_err(|e| format!("Error deserializing list for id {}: {}", self.id, e))?;
                *self.list.write() = list;
                Ok(())
            }
            ListHeader::Set(idx) => {
                let value: T = deserialize(data).map_err(|e| {
                    format!("Error deserializing list item for id {}: {}", self.id, e)
                })?;
                let mut list = self.list.write();
                let idx = idx as usize;
                if idx < list.len() {
                    list[idx] = value;
                }
                Ok(())
            }
            ListHeader::Add => {
                let value: T = deserialize(data).map_err(|e| {
                    format!("Error deserializing list item for id {}: {}", self.id, e)
                })?;
                self.list.write().push(value);
                Ok(())
            }
            ListHeader::Remove(idx) => {
                let mut list = self.list.write();
                let idx = idx as usize;
                if idx < list.len() {
                    list.remove(idx);
                }
                Ok(())
            }
        }
    }
}
