use parking_lot::Mutex;
use std::collections::{VecDeque, hash_map::Entry};
use std::sync::Arc;

use egui_states_core::nohash::{NoHashMap, NoHashSet};

use crate::event::Event;
use crate::python_convert::ToPython;

enum Signal {
    Single(Box<dyn ToPython + Sync + Send>),
    Multi(VecDeque<Box<dyn ToPython + Sync + Send>>),
}

struct OrderedMap {
    values: NoHashMap<u32, Signal>,
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

    fn pop(&mut self, id: u32) -> Option<Box<dyn ToPython + Sync + Send>> {
        match self.values.get_mut(&id) {
            None => None,
            Some(Signal::Single(_)) => match self.values.remove(&id).unwrap() {
                Signal::Single(v) => Some(v),
                _ => unreachable!(),
            },
            Some(Signal::Multi(queue)) => queue.pop_front(),
        }
    }

    fn pop_first(&mut self) -> Option<(u32, Box<dyn ToPython + Sync + Send>)> {
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

    fn set_to_multi(&mut self, id: u32) {
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

    fn set_to_single(&mut self, id: u32) {
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
    blocked_list: NoHashSet<u32>, // ids blocked by some thread
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
        }
    }

    fn set(&mut self, id: u32, value: Box<dyn ToPython + Sync + Send>, event: &Event) {
        self.values.insert(id, value);
        if !self.blocked_list.contains(&id) {
            event.set_one();
        }
    }

    fn get(&mut self, last_id: Option<u32>) -> Option<(u32, Box<dyn ToPython + Send + Sync>)> {
        match last_id {
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
        }
    }
}

#[derive(Clone)]
pub(crate) struct ChangedValues {
    event: Event,
    values: Arc<Mutex<ChangedInner>>,
}

impl ChangedValues {
    pub fn new() -> Self {
        Self {
            event: Event::new(),
            values: Arc::new(Mutex::new(ChangedInner::new())),
        }
    }

    pub fn set(&self, id: u32, value: impl ToPython + Sync + Send + 'static) {
        let value = Box::new(value);
        self.values.lock().set(id, value, &self.event);
    }

    #[allow(dead_code)]
    pub(crate) fn debug(&self, message: impl ToString) {
        self.set(0, (0, message.to_string()));
    }

    pub(crate) fn info(&self, message: impl ToString) {
        self.set(0, (1, message.to_string()));
    }

    pub(crate) fn warning(&self, message: impl ToString) {
        self.set(0, (2, message.to_string()));
    }

    pub(crate) fn error(&self, message: impl ToString) {
        self.set(0, (3, message.to_string()));
    }

    pub fn wait_changed_value(
        &self,
        last_id: Option<u32>,
    ) -> (u32, Box<dyn ToPython + Send + Sync>) {
        loop {
            if let Some(val) = self.values.lock().get(last_id) {
                return val;
            }
            self.event.wait_lock();
        }
    }

    pub fn set_to_multi(&self, id: u32) {
        self.values.lock().values.set_to_multi(id);
    }

    pub fn set_to_single(&self, id: u32) {
        self.values.lock().values.set_to_single(id);
    }
}
