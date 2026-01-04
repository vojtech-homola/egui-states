use std::collections::{VecDeque, hash_map::Entry};
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use egui_states_core::generate_value_id;
use egui_states_core::nohash::{NoHashMap, NoHashSet};
use egui_states_core::serialization::{FastVec, serialize_to_data};

use crate::event::Event;

enum Signal {
    Single(Bytes),
    Multi(VecDeque<Bytes>),
}

struct OrderedMap {
    values: NoHashMap<u64, Signal>,
    indexes: VecDeque<u64>,
}

impl OrderedMap {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            indexes: VecDeque::new(),
        }
    }

    fn clear(&mut self) {
        self.values.clear();
        self.indexes.clear();
    }

    fn insert(&mut self, id: u64, value: Bytes) {
        let entry = self.values.entry(id);
        match entry {
            Entry::Vacant(e) => {
                e.insert(Signal::Single(value));
            }
            Entry::Occupied(mut e) => match e.get_mut() {
                Signal::Single(v) => *v = value,
                Signal::Multi(v) => v.push_back(value),
            },
        }
        self.indexes.push_back(id);
    }

    fn pop(&mut self, id: u64) -> Option<Bytes> {
        match self.values.get_mut(&id) {
            None => None,
            Some(Signal::Single(_)) => match self.values.remove(&id).unwrap() {
                Signal::Single(v) => Some(v),
                _ => unreachable!(),
            },
            Some(Signal::Multi(queue)) => queue.pop_front(),
        }
    }

    fn pop_first(&mut self) -> Option<(u64, Bytes)> {
        while let Some(id) = self.indexes.pop_front() {
            if let Some(value) = self.pop(id) {
                return Some((id, value));
            } else {
                while self.indexes.front().map_or(false, |next| *next == id) {
                    self.indexes.pop_front();
                }
            }
        }
        None
    }

    fn set_to_multi(&mut self, id: u64) {
        if let Some(signal) = self.values.remove(&id) {
            let res = match signal {
                Signal::Single(v) => {
                    let mut vec = VecDeque::new();
                    vec.push_back(v);
                    vec
                }
                Signal::Multi(vec) => vec,
            };
            self.values.insert(id, Signal::Multi(res));
        } else {
            self.values.insert(id, Signal::Multi(VecDeque::new()));
        }
    }

    fn set_to_single(&mut self, id: u64) {
        if let Some(signal) = self.values.remove(&id) {
            let res = match signal {
                Signal::Single(v) => Some(v),
                Signal::Multi(mut vec) => vec.pop_back(),
            };

            if let Some(res) = res {
                self.values.insert(id, Signal::Single(res));
            }
        }
    }
}

struct ChangedInner {
    values: OrderedMap,           // values not blocked
    blocked_list: NoHashSet<u64>, // ids blocked by some thread
    registered: NoHashSet<u64>,   // ids which are registered to be signaled
}

/*
    Getting signals value in that way that if there is new signal with the same id which is
    currently processed, it will wait for the same thread. So id is processed in order.
*/
impl ChangedInner {
    fn new() -> Self {
        Self {
            values: OrderedMap::new(),
            blocked_list: NoHashSet::default(),
            registered: NoHashSet::default(),
        }
    }

    fn clear(&mut self) {
        self.values.clear();
        self.blocked_list.clear();
    }

    fn set(&mut self, id: u64, value: Bytes, event: &Event) {
        self.values.insert(id, value);
        if !self.blocked_list.contains(&id) {
            event.set_one();
        }
    }

    fn get(&mut self, last_id: Option<u64>) -> Option<(u64, Bytes)> {
        let res = match last_id {
            // previous call was made
            Some(last_id) => {
                if self.blocked_list.contains(&last_id) {
                    let val = self.values.pop(last_id);
                    match val {
                        Some(v) => Some((last_id, v)),
                        None => {
                            let val = self.values.pop_first();
                            self.blocked_list.remove(&last_id);

                            if let Some((id, _)) = val {
                                self.blocked_list.insert(id);
                            }
                            val
                        }
                    }
                } else {
                    let val = self.values.pop_first();
                    if let Some((id, _)) = val {
                        self.blocked_list.insert(id);
                    }
                    val
                }
            }
            // this is first time
            None => {
                let val = self.values.pop_first();
                if let Some((id, _)) = val {
                    self.blocked_list.insert(id);
                }
                val
            }
        };

        match &res {
            Some((id, _)) => match self.registered.contains(id) {
                true => res,
                false => None,
            },
            None => None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct SignalsManager {
    event: Event,
    values: Arc<Mutex<ChangedInner>>,
    logging_id: u64,
}

impl SignalsManager {
    pub(crate) fn new() -> Self {
        let logging_id = generate_value_id("__egui_states_logging");
        Self {
            event: Event::new(),
            values: Arc::new(Mutex::new(ChangedInner::new())),
            logging_id,
        }
    }

    pub(crate) fn set(&self, id: u64, value: Bytes) {
        self.values.lock().set(id, value, &self.event);
    }

    pub(crate) fn reset(&self) {
        self.values.lock().clear();
    }

    fn serialize_message(level: u8, text: impl ToString) -> Bytes {
        let data = text.to_string();
        let message = FastVec::<64>::new();
        let message = serialize_to_data(&level, message);
        let message = serialize_to_data(&data, message);
        message.to_bytes()
    }

    #[allow(dead_code)]
    pub(crate) fn debug(&self, message: impl ToString) {
        let data = Self::serialize_message(0u8, message);
        self.set(self.logging_id, data);
    }

    pub(crate) fn info(&self, message: impl ToString) {
        let data = Self::serialize_message(1u8, message);
        self.set(self.logging_id, data);
    }

    pub(crate) fn warning(&self, message: impl ToString) {
        let data = Self::serialize_message(2u8, message);
        self.set(self.logging_id, data);
    }

    pub(crate) fn error(&self, message: impl ToString) {
        let data = Self::serialize_message(3u8, message);
        self.set(self.logging_id, data);
    }

    pub(crate) fn wait_changed_value(&self, last_id: Option<u64>) -> (u64, Bytes) {
        loop {
            if let Some(val) = self.values.lock().get(last_id) {
                return val;
            }
            self.event.wait_lock();
        }
    }

    pub(crate) fn set_register(&self, id: u64, register: bool) {
        if register {
            self.values.lock().registered.insert(id);
        } else {
            self.values.lock().registered.remove(&id);
        }
    }

    pub(crate) fn set_to_multi(&self, id: u64) {
        self.values.lock().values.set_to_multi(id);
    }

    pub(crate) fn set_to_single(&self, id: u64) {
        self.values.lock().values.set_to_single(id);
    }

    pub(crate) fn get_logging_id(&self) -> u64 {
        self.logging_id
    }
}
