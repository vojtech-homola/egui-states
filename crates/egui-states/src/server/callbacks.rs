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

pub(super) type Callback = Arc<dyn Fn(Bytes) -> Result<()> + Send + Sync + 'static>;

#[derive(Clone)]
pub(super) struct CallbackEntry {
    pub(super) id: u64,
    pub(super) callback: Callback,
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
    ) -> u64 {
        let callback_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut callbacks = self.callbacks.lock();
        callbacks.entry(value_id).or_default().push(CallbackEntry {
            id: callback_id,
            callback,
        });
        signals.set_register(value_id, true);
        callback_id
    }

    pub(super) fn remove(&self, signals: &CoreSignalsManager, value_id: u64, callback_id: u64) {
        let mut callbacks = self.callbacks.lock();
        if let Some(entries) = callbacks.get_mut(&value_id) {
            entries.retain(|entry| entry.id != callback_id);
            if entries.is_empty() {
                callbacks.remove(&value_id);
                signals.set_register(value_id, false);
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
