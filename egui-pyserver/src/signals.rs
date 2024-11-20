use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use egui_pysync::event::Event;
use egui_pysync::{NoHashMap, NoHashSet};

use crate::ToPython;

struct OrderedMap {
    values: NoHashMap<u32, Box<dyn ToPython + Sync + Send>>,
    indexes: VecDeque<u32>,
}

impl OrderedMap {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            indexes: VecDeque::new(),
        }
    }

    fn insert(&mut self, id: u32, value: Box<dyn ToPython + Sync + Send>) {
        self.values.insert(id, value);
        self.indexes.push_back(id);
    }

    fn pop_first(&mut self) -> Option<(u32, Box<dyn ToPython + Sync + Send>)> {
        for _ in 0..self.indexes.len() {
            let id = self.indexes.pop_front().unwrap();
            if let Some(value) = self.values.remove(&id) {
                return Some((id, value));
            }
        }
        None
    }
}

struct ChnegedInner {
    values: OrderedMap,                                         // values not blocked
    blocked: NoHashMap<u32, Box<dyn ToPython + Sync + Send>>, // values blocked by some thread
    block_list: NoHashSet<u32>,                                 // ids blocked by some thread
    threads_last: NoHashMap<u32, u32>,                          // cache last id for each thread
}

/*
    Getting signals value in that way that if thare is new signal with the same id which is
    currently processed, it will wait for the same thread. So on id is processed in order.
*/
impl ChnegedInner {
    fn new() -> Self {
        Self {
            values: OrderedMap::new(),
            blocked: NoHashMap::default(),
            block_list: NoHashSet::default(),
            threads_last: NoHashMap::default(),
        }
    }

    fn set(&mut self, id: u32, value: Box<dyn ToPython + Sync + Send>, event: &Event) {
        if self.block_list.contains(&id) {
            self.blocked.insert(id, value);
        } else {
            self.values.insert(id, value);
            event.set_one();
        }
    }

    fn get(&mut self, thread_id: u32) -> Option<(u32, Box<dyn ToPython + Send + Sync>)> {
        match self.threads_last.get(&thread_id) {
            // previous call was made
            Some(last_id) => {
                if self.block_list.contains(last_id) {
                    let val = self.blocked.remove(last_id);
                    match val {
                        Some(v) => Some((*last_id, v)),
                        None => {
                            let val = self.values.pop_first();
                            self.block_list.remove(last_id);

                            if let Some(ref v) = val {
                                self.threads_last.insert(thread_id, v.0);
                                self.block_list.insert(v.0);
                            }
                            val
                        }
                    }
                } else {
                    let val = self.values.pop_first();
                    if let Some(ref v) = val {
                        self.threads_last.insert(thread_id, v.0);
                        self.block_list.insert(v.0);
                    }
                    val
                }
            }
            // this is first time
            None => {
                let val = self.values.pop_first();
                if let Some(ref v) = val {
                    self.threads_last.insert(thread_id, v.0);
                    self.block_list.insert(v.0);
                }
                val
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct ChangedValues {
    event: Event,
    values: Arc<Mutex<ChnegedInner>>,
}

impl ChangedValues {
    pub fn new() -> Self {
        Self {
            event: Event::new(),
            values: Arc::new(Mutex::new(ChnegedInner::new())),
        }
    }

    pub fn set(&self, id: u32, value: impl ToPython + Sync + Send + 'static) {
        let value = Box::new(value);
        self.values.lock().unwrap().set(id, value, &self.event);
    }

    pub fn wait_changed_value(&self, thread_id: u32) -> (u32, Box<dyn ToPython + Send + Sync>) {
        loop {
            if let Some(val) = self.values.lock().unwrap().get(thread_id) {
                return val;
            }
            self.event.wait_lock();
        }
    }
}
