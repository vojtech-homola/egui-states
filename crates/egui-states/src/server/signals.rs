use std::collections::{VecDeque, hash_map::Entry};
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::hashing::{NoHashMap, NoHashSet};
use crate::serialization::{FastVec, serialize, serialize_to_data};
use crate::server::event::Event;

pub(crate) const LOGGING_ID: u64 = 0;
pub(crate) const ON_CONNECT_ID: u64 = 1;
pub(crate) const ON_DISCONNECT_ID: u64 = 2;
pub(crate) const CLIENT_MESSAGE_ID: u64 = 3;

enum Signal {
    Single(Bytes),
    Queue(VecDeque<Bytes>),
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
        self.values.retain(|id, _| *id <= 9);
        self.indexes.retain(|id| *id <= 9);
    }

    fn insert(&mut self, id: u64, value: Bytes) {
        let entry = self.values.entry(id);
        match entry {
            Entry::Vacant(e) => {
                e.insert(Signal::Single(value));
            }
            Entry::Occupied(mut e) => match e.get_mut() {
                Signal::Single(v) => *v = value,
                Signal::Queue(v) => v.push_back(value),
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
            Some(Signal::Queue(queue)) => queue.pop_front(),
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

    fn set_to_queue(&mut self, id: u64) {
        if let Some(signal) = self.values.remove(&id) {
            let res = match signal {
                Signal::Single(v) => {
                    let mut vec = VecDeque::new();
                    vec.push_back(v);
                    vec
                }
                Signal::Queue(vec) => vec,
            };
            self.values.insert(id, Signal::Queue(res));
        } else {
            self.values.insert(id, Signal::Queue(VecDeque::new()));
        }
    }

    fn set_to_single(&mut self, id: u64) {
        if let Some(signal) = self.values.remove(&id) {
            let res = match signal {
                Signal::Single(v) => Some(v),
                Signal::Queue(mut vec) => vec.pop_back(),
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
        self.blocked_list.retain(|id| *id <= 9);
    }

    fn set(&mut self, id: u64, value: Bytes, event: &Event) {
        self.values.insert(id, value);
        if !self.blocked_list.contains(&id) {
            event.set_one();
        }
    }

    fn get(&mut self, last_id: Option<u64>) -> Option<(u64, Bytes)> {
        match last_id {
            // previous call was made
            Some(last_id) => {
                if self.blocked_list.contains(&last_id) {
                    let val = self.values.pop(last_id);
                    match val {
                        Some(v) => Some((last_id, v)),
                        None => {
                            self.blocked_list.remove(&last_id);
                            let val = self.values.pop_first();
                            if let Some((id, _)) = &val {
                                if self.registered.contains(id) {
                                    self.blocked_list.insert(*id);
                                    return val;
                                }
                            }
                            return None;
                        }
                    }
                } else {
                    let val = self.values.pop_first();
                    if let Some((id, _)) = &val {
                        if self.registered.contains(id) {
                            self.blocked_list.insert(*id);
                            return val;
                        }
                    }
                    return None;
                }
            }
            // this is first time
            None => {
                let val = self.values.pop_first();
                if let Some((id, _)) = &val {
                    if self.registered.contains(id) {
                        self.blocked_list.insert(*id);
                        return val;
                    }
                }
                return None;
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct SignalsManager {
    event: Event,
    values: Arc<Mutex<ChangedInner>>,
}

impl SignalsManager {
    pub(crate) fn new() -> Self {
        Self {
            event: Event::new(),
            values: Arc::new(Mutex::new(ChangedInner::new())),
        }
    }

    pub(crate) fn set(&self, id: u64, value: Bytes) {
        self.values.lock().set(id, value, &self.event);
    }

    pub(crate) fn reset(&self) {
        self.values.lock().clear();
    }

    fn serialize_message(level: u8, text: impl ToString) -> Result<Bytes, ()> {
        let data = text.to_string();
        let mut message = FastVec::<64>::new();
        serialize_to_data(&level, &mut message)?;
        serialize_to_data(&data, &mut message)?;
        Ok(message.to_bytes())
    }

    #[allow(dead_code)]
    #[inline]
    pub(crate) fn debug(&self, message: impl ToString) {
        if let Ok(data) = Self::serialize_message(0u8, message) {
            self.set(LOGGING_ID, data);
        }
    }

    #[inline]
    pub(crate) fn info(&self, message: impl ToString) {
        if let Ok(data) = Self::serialize_message(1u8, message) {
            self.set(LOGGING_ID, data);
        }
    }

    #[inline]
    pub(crate) fn warning(&self, message: impl ToString) {
        if let Ok(data) = Self::serialize_message(2u8, message) {
            self.set(LOGGING_ID, data);
        }
    }

    #[inline]
    pub(crate) fn error(&self, message: impl ToString) {
        if let Ok(data) = Self::serialize_message(3u8, message) {
            self.set(LOGGING_ID, data);
        }
    }

    #[inline]
    pub(crate) fn on_connect(&self, peer_addr: String) {
        if let Ok(result) = serialize::<String, 32>(&peer_addr) {
            self.set(ON_CONNECT_ID, result.to_bytes());
        }
    }

    #[inline]
    pub(crate) fn on_disconnect(&self) {
        self.set(ON_DISCONNECT_ID, Bytes::new());
    }

    #[inline]
    pub(crate) fn client_message(&self, message: Bytes) {
        self.set(CLIENT_MESSAGE_ID, message);
    }

    pub(crate) fn wait_changed_value(&self, last_id: Option<u64>) -> (u64, Bytes) {
        loop {
            if let Some(val) = self.values.lock().get(last_id) {
                return val;
            }
            self.event.wait_clear();
        }
    }

    pub(crate) fn set_register(&self, id: u64, register: bool) {
        if register {
            self.values.lock().registered.insert(id);
        } else {
            let mut w = self.values.lock();
            w.registered.remove(&id);
            w.blocked_list.remove(&id);
        }
    }

    pub(crate) fn set_to_queue(&self, id: u64) {
        self.values.lock().values.set_to_queue(id);
    }

    pub(crate) fn set_to_single(&self, id: u64) {
        self.values.lock().values.set_to_single(id);
    }
}
