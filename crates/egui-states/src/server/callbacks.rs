use std::collections::HashMap;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicU64, Ordering},
};

use bytes::Bytes;
use parking_lot::Mutex;

use crate::server_core::signals::SignalsManager as CoreSignalsManager;

use super::Result;
use super::state_server::ServerInner;

pub(super) type Callback = Arc<dyn Fn(Bytes, Option<Bytes>) -> Result<()> + Send + Sync + 'static>;

#[derive(Clone)]
pub(super) struct CallbackEntry {
    pub(super) id: u64,
    pub(super) callback: Callback,
    /// Whether this callback needs the replaced value. Registration passes the
    /// aggregate over all entries for a value, so the previous value is only kept
    /// while at least one callback asks for it.
    pub(super) wants_previous: bool,
}

pub(super) struct CallbackRegistry {
    next_id: AtomicU64,
    callbacks: Mutex<HashMap<u64, Vec<CallbackEntry>>>,
}

impl CallbackRegistry {
    pub(super) fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            callbacks: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn add(
        &self,
        signals: &CoreSignalsManager,
        value_id: u64,
        callback: Callback,
        wants_previous: bool,
    ) -> u64 {
        let callback_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut callbacks = self.callbacks.lock();
        let entries = callbacks.entry(value_id).or_default();
        entries.push(CallbackEntry {
            id: callback_id,
            callback,
            wants_previous,
        });
        signals.set_register(value_id, true, any_wants_previous(entries));
        callback_id
    }

    pub(super) fn remove(&self, signals: &CoreSignalsManager, value_id: u64, callback_id: u64) {
        let mut callbacks = self.callbacks.lock();
        if let Some(entries) = callbacks.get_mut(&value_id) {
            entries.retain(|entry| entry.id != callback_id);
            if entries.is_empty() {
                callbacks.remove(&value_id);
                signals.set_register(value_id, false, false);
            } else {
                // Dropping the last previous-value callback stops the previous value
                // from being carried, so the flag has to be recomputed here too.
                signals.set_register(value_id, true, any_wants_previous(entries));
            }
        }
    }

    pub(super) fn get(&self, value_id: u64) -> Vec<CallbackEntry> {
        self.callbacks
            .lock()
            .get(&value_id)
            .cloned()
            .unwrap_or_default()
    }
}

fn any_wants_previous(entries: &[CallbackEntry]) -> bool {
    entries.iter().any(|entry| entry.wants_previous)
}

#[must_use = "the callback is disconnected when its handle is dropped"]
pub struct CallbackHandle {
    pub(super) server: Weak<ServerInner>,
    pub(super) value_id: u64,
    pub(super) callback_id: u64,
}

impl CallbackHandle {
    /// Disconnects the callback.
    ///
    /// Calling this more than once is harmless.
    pub fn disconnect(&self) {
        if let Some(server) = self.server.upgrade() {
            server
                .callbacks
                .remove(&server.signals, self.value_id, self.callback_id);
        }
    }
}

impl Drop for CallbackHandle {
    fn drop(&mut self) {
        self.disconnect();
    }
}
