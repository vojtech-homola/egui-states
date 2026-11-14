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

/// Callbacks per value. A value's entries are shared with the workers that
/// run them, so an event takes a reference instead of copying the list;
/// registration, which is rare, replaces the list.
pub(super) struct CallbackRegistry {
    next_id: AtomicU64,
    callbacks: Mutex<HashMap<u64, Arc<[CallbackEntry]>>>,
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
        let mut entries = callbacks
            .get(&value_id)
            .map_or_else(Vec::new, |entries| entries.to_vec());
        entries.push(CallbackEntry {
            id: callback_id,
            callback,
            wants_previous,
        });
        signals.set_register(value_id, true, any_wants_previous(&entries));
        callbacks.insert(value_id, entries.into());
        callback_id
    }

    pub(super) fn remove(&self, signals: &CoreSignalsManager, value_id: u64, callback_id: u64) {
        let mut callbacks = self.callbacks.lock();
        if let Some(entries) = callbacks.get(&value_id) {
            let entries: Vec<CallbackEntry> = entries
                .iter()
                .filter(|entry| entry.id != callback_id)
                .cloned()
                .collect();
            if entries.is_empty() {
                callbacks.remove(&value_id);
                signals.set_register(value_id, false, false);
            } else {
                // Dropping the last previous-value callback stops the previous value
                // from being carried, so the flag has to be recomputed here too.
                signals.set_register(value_id, true, any_wants_previous(&entries));
                callbacks.insert(value_id, entries.into());
            }
        }
    }

    /// Drops every callback and unregisters its value, which also discards the
    /// value's pending changes.
    ///
    /// Used at teardown: states can outlive the server and share its signals,
    /// so a value left registered would keep queueing changes nobody drains.
    /// Handles still alive cannot do this themselves, because their server is
    /// gone.
    pub(super) fn clear(&self, signals: &CoreSignalsManager) {
        let callbacks = std::mem::take(&mut *self.callbacks.lock());
        for value_id in callbacks.keys() {
            signals.set_register(*value_id, false, false);
        }
    }

    pub(super) fn get(&self, value_id: u64) -> Arc<[CallbackEntry]> {
        self.callbacks
            .lock()
            .get(&value_id)
            .cloned()
            .unwrap_or_else(|| Arc::new([]))
    }
}

fn any_wants_previous(entries: &[CallbackEntry]) -> bool {
    entries.iter().any(|entry| entry.wants_previous)
}

#[must_use = "the callback is disconnected when its handle is dropped"]
/// Owning handle for a registered server callback.
///
/// Dropping or explicitly disconnecting the handle unregisters the callback.
/// Keep it alive for as long as the callback should remain connected.
///
/// The handle does not keep the server alive. A callback connected after its
/// server was dropped is discarded, and its handle does nothing.
pub struct CallbackHandle {
    pub(super) server: Weak<ServerInner>,
    pub(super) value_id: u64,
    pub(super) callback_id: u64,
    /// Runs once when the callback is disconnected.
    pub(super) cleanup: parking_lot::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl CallbackHandle {
    /// A handle for a callback that was never registered because its server
    /// had already been dropped. Disconnecting it does nothing.
    pub(super) fn detached(value_id: u64) -> Self {
        Self {
            server: Weak::new(),
            value_id,
            callback_id: 0,
            cleanup: parking_lot::Mutex::new(None),
        }
    }

    /// Runs `cleanup` when the callback is disconnected.
    pub(super) fn on_disconnect(self, cleanup: impl FnOnce() + Send + 'static) -> Self {
        *self.cleanup.lock() = Some(Box::new(cleanup));
        self
    }

    /// Disconnects the callback.
    ///
    /// Calling this more than once is harmless.
    pub fn disconnect(&self) {
        if let Some(server) = self.server.upgrade() {
            server
                .callbacks
                .remove(&server.signals, self.value_id, self.callback_id);
        }
        if let Some(cleanup) = self.cleanup.lock().take() {
            cleanup();
        }
    }
}

impl Drop for CallbackHandle {
    fn drop(&mut self) {
        self.disconnect();
    }
}
