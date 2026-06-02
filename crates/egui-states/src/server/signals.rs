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

struct ChangedInner {
    values: NoHashMap<u64, Signal>, // stored signals
    indexes: Vec<u64>,              // scheduling order; may contain stale duplicate IDs
    blocked_list: NoHashSet<u64>,   // ids blocked by some thread
    registered: NoHashSet<u64>,     // ids which are registered to be signaled
}

impl ChangedInner {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            indexes: Vec::new(),
            blocked_list: NoHashSet::default(),
            registered: NoHashSet::default(),
        }
    }

    fn clear(&mut self) {
        self.values.retain(|id, _| *id <= 9);
        self.indexes.retain(|id| *id <= 9);
        self.blocked_list.retain(|id| *id <= 9);
    }

    fn set(&mut self, id: u64, value: Bytes, event: &Event) {
        if !self.registered.contains(&id) {
            return;
        }

        self.indexes.push(id);
        match self.values.entry(id) {
            Entry::Vacant(e) => {
                e.insert(Signal::Single(value));
            }
            Entry::Occupied(mut e) => match e.get_mut() {
                Signal::Single(v) => *v = value,
                Signal::Queue(v) => v.push_back(value),
            },
        }

        if !self.blocked_list.contains(&id) {
            event.set_one();
        }
    }

    fn get_id(&mut self, id: u64) -> Option<Bytes> {
        match self.values.remove(&id) {
            None => None,
            Some(Signal::Single(v)) => {
                self.indexes.retain(|&single_id| single_id != id);
                Some(v)
            }
            Some(Signal::Queue(mut queue)) => {
                let val = queue.pop_front();
                self.values.insert(id, Signal::Queue(queue));
                val
            }
        }
    }

    fn get(&mut self, last_id: Option<u64>) -> Option<(u64, Bytes)> {
        if let Some(last_id) = last_id {
            self.blocked_list.remove(&last_id);
        }

        let mut pos = 0;
        while pos < self.indexes.len() {
            let id = self.indexes[pos];

            if self.blocked_list.contains(&id) {
                pos += 1;
                continue;
            }

            self.indexes.remove(pos);

            if !self.registered.contains(&id) {
                self.values.remove(&id);
                continue;
            }

            if let Some(value) = self.get_id(id) {
                self.blocked_list.insert(id);
                return Some((id, value));
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
            w.values.remove(&id);
            w.indexes.retain(|queued_id| *queued_id != id);
        }
    }

    pub(crate) fn set_to_queue(&self, id: u64) {
        self.values.lock().set_to_queue(id);
    }

    pub(crate) fn set_to_single(&self, id: u64) {
        self.values.lock().set_to_single(id);
    }
}
