use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use egui_pysync_common::event::Event;
use egui_pysync_common::values::ValueMessage;

struct OrderedMap {
    values: HashMap<u32, ValueMessage>,
    indexes: VecDeque<u32>,
}

impl OrderedMap {
    fn new() -> Self {
        Self {
            values: HashMap::new(),
            indexes: VecDeque::new(),
        }
    }

    fn insert(&mut self, id: u32, value: ValueMessage) {
        self.values.insert(id, value);
        self.indexes.push_back(id);
    }

    fn pop_first(&mut self) -> Option<(u32, ValueMessage)> {
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
    values: OrderedMap,                  // values not blocked
    blocked: HashMap<u32, ValueMessage>, // values blocked by some thread
    block_list: HashSet<u32>,            // ids blocked by some thread
    threads_last: HashMap<u32, u32>,     // cache last id for each thread
}

/*
    Getting signals value in that way that if thare is new signal with the same id which is
    currently processed, it will wait for the same thread. So on id is processed in order.
*/
impl ChnegedInner {
    fn new() -> Self {
        Self {
            values: OrderedMap::new(),
            blocked: HashMap::new(),
            block_list: HashSet::new(),
            threads_last: HashMap::new(),
        }
    }

    fn set(&mut self, id: u32, value: ValueMessage, event: &Event) {
        if self.block_list.contains(&id) {
            self.blocked.insert(id, value);
        } else {
            self.values.insert(id, value);
            event.set_one();
        }
    }

    fn get(&mut self, thread_id: u32) -> Option<(u32, ValueMessage)> {
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
pub struct ChangedValues {
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

    pub fn set(&self, id: u32, value: ValueMessage) {
        self.values.lock().unwrap().set(id, value, &self.event);
    }

    pub fn wait_changed_value(&self, thread_id: u32) -> (u32, ValueMessage) {
        loop {
            if let Some(val) = self.values.lock().unwrap().get(thread_id) {
                return val;
            }
            self.event.wait_lock();
        }
    }
}
