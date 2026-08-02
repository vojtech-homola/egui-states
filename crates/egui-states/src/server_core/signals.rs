use std::collections::{VecDeque, hash_map::Entry};
use std::sync::Arc;
#[cfg(feature = "server")]
use std::sync::atomic::AtomicBool;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::event::Event;
use crate::hashing::{NoHashMap, NoHashSet};
use crate::serialization::{FastVec, serialize, serialize_to_data};

pub(crate) const LOGGING_ID: u64 = 0;
pub(crate) const ON_CONNECT_ID: u64 = 1;
pub(crate) const ON_DISCONNECT_ID: u64 = 2;
pub(crate) const CLIENT_MESSAGE_ID: u64 = 3;

/// A stored change. `previous` is only ever set by a `Value`, which is the sole
/// state that keeps a value to be replaced; a `Signal` stores nothing, so it has
/// no previous value to report.
struct Change {
    value: Bytes,
    previous: Option<Bytes>,
}

enum Signal {
    Single(Change),
    Queue(VecDeque<Change>),
}

struct ChangedInner {
    values: NoHashMap<u64, Signal>,   // stored signals
    indexes: Vec<u64>,                // scheduling order; may contain stale duplicate IDs
    blocked_list: NoHashSet<u64>,     // ids blocked by some thread
    registered: NoHashMap<u64, bool>, // ids registered to be signaled -> wants previous value
}

impl ChangedInner {
    fn new() -> Self {
        Self {
            values: NoHashMap::default(),
            indexes: Vec::new(),
            blocked_list: NoHashSet::default(),
            registered: NoHashMap::default(),
        }
    }

    fn clear(&mut self) {
        self.values.retain(|id, _| *id <= 9);
        self.indexes.retain(|id| *id <= 9);
        self.blocked_list.retain(|id| *id <= 9);
    }

    fn set(&mut self, id: u64, value: Bytes, previous: Option<Bytes>, event: &Event) {
        if !self.registered.contains_key(&id) {
            return;
        }

        self.indexes.push(id);
        match self.values.entry(id) {
            Entry::Vacant(e) => {
                e.insert(Signal::Single(Change { value, previous }));
            }
            Entry::Occupied(mut e) => match e.get_mut() {
                Signal::Single(v) => {
                    // A pending single change is overwritten rather than queued, so keep
                    // the oldest unconsumed previous value. That way `previous` is always
                    // the value the consumer was last told about: for a -> b -> c where
                    // only c is delivered, it sees (c, a) and can still diff against its
                    // own view. Reporting b would name a value it never received.
                    let previous = v.previous.take().or(previous);
                    *v = Change { value, previous };
                }
                Signal::Queue(v) => v.push_back(Change { value, previous }),
            },
        }

        if !self.blocked_list.contains(&id) {
            event.set_one();
        }
    }

    fn get_id(&mut self, id: u64) -> Option<Change> {
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

    fn get(&mut self, last_id: Option<u64>) -> Option<(u64, Bytes, Option<Bytes>)> {
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

            let Some(&wants_previous) = self.registered.get(&id) else {
                self.values.remove(&id);
                continue;
            };

            if let Some(change) = self.get_id(id) {
                self.blocked_list.insert(id);
                let previous = match wants_previous {
                    true => change.previous,
                    false => None,
                };
                return Some((id, change.value, previous));
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
                // Only the newest queued change survives, so it inherits the oldest
                // unconsumed previous -- same reasoning as the coalescing branch in
                // `set`. Popping first is what makes that correct: afterwards the
                // front is necessarily an older change, or the queue held at most one
                // and there is nothing older to inherit.
                Signal::Queue(mut vec) => vec.pop_back().map(|mut newest| {
                    if let Some(oldest) = vec.front_mut().and_then(|c| c.previous.take()) {
                        newest.previous = Some(oldest);
                    }
                    newest
                }),
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
        self.values.lock().set(id, value, None, &self.event);
    }

    /// Like [`Self::set`], but records the value being replaced. Only a `Value` has
    /// one; the previous value is delivered to consumers that registered for it (see
    /// [`Self::set_register`]) and dropped for those that did not.
    pub(crate) fn set_with_previous(&self, id: u64, value: Bytes, previous: Bytes) {
        self.values
            .lock()
            .set(id, value, Some(previous), &self.event);
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

    #[cfg(feature = "python")]
    pub(crate) fn wait_changed_value(
        &self,
        mut last_id: Option<u64>,
    ) -> (u64, Bytes, Option<Bytes>) {
        loop {
            if let Some(val) = self.values.lock().get(last_id.take()) {
                return val;
            }
            self.event.wait_clear();
        }
    }

    #[cfg(feature = "server")]
    pub(crate) fn try_changed_value(
        &self,
        last_id: Option<u64>,
    ) -> Option<(u64, Bytes, Option<Bytes>)> {
        self.values.lock().get(last_id)
    }

    // #[cfg(feature = "python")]
    // pub(crate) fn wait_for_change(&self) {
    //     self.event.wait_clear();
    // }

    #[cfg(feature = "server")]
    pub(crate) fn wait_for_change_until(&self, stop: &AtomicBool) -> bool {
        self.event.wait_clear_until(stop)
    }

    #[cfg(feature = "server")]
    pub(crate) fn wake_waiters(&self) {
        self.event.set();
    }

    /// Releases an id that was handed out but will never be passed back as
    /// `last_id`, because the caller failed before it could use the value.
    #[cfg(feature = "python")]
    pub(crate) fn release(&self, id: u64) {
        self.values.lock().blocked_list.remove(&id);
        // Anything that arrived while the id was blocked did not set the event
        // (`ChangedInner::set` skips blocked ids), so wake a waiter to drain it.
        // A spurious wake-up is harmless: the waiter just finds nothing.
        self.event.set_one();
    }

    /// Registers `id` to be signaled. `with_previous` asks for the replaced value to
    /// be delivered alongside the new one; consumers that do not ask never see it, so
    /// they never pay to decode it. Callers with several consumers per id must pass
    /// the aggregate -- `true` if any of them wants the previous value.
    pub(crate) fn set_register(&self, id: u64, register: bool, with_previous: bool) {
        if register {
            self.values.lock().registered.insert(id, with_previous);
        } else {
            let mut w = self.values.lock();
            w.registered.remove(&id);
            if let Some(Signal::Queue(mut q)) = w.values.remove(&id) {
                q.clear();
                w.values.insert(id, Signal::Queue(q));
            }
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

#[cfg(all(test, feature = "python"))]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    /// A caller that fails before passing an id back as `last_id` must not
    /// strand it: `release` is what keeps signals queued behind it reachable.
    #[test]
    fn release_frees_an_id_that_was_never_passed_back() {
        let manager = SignalsManager::new();
        manager.set_register(5, true, false);
        manager.set_register(9, true, false);

        manager.set(5, Bytes::from_static(b"first"));
        assert_eq!(manager.wait_changed_value(None).0, 5);

        // A second signal for 5 queues silently -- it is blocked, so no wake-up
        // is sent and only 9 is reachable.
        manager.set(5, Bytes::from_static(b"stranded"));
        manager.set(9, Bytes::from_static(b"other"));
        assert_eq!(
            manager.wait_changed_value(None).0,
            9,
            "5 must stay blocked while its holder has not released it"
        );

        // Nothing else will ever release 5, since the caller that took it
        // failed before it could pass it back.
        manager.release(5);

        // Bounded, so a regression fails instead of blocking forever.
        let (sender, receiver) = mpsc::channel();
        let waiter = manager.clone();
        thread::spawn(move || {
            let _ = sender.send(waiter.wait_changed_value(None).0);
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).ok(),
            Some(5),
            "the signal queued behind 5 should be reachable again"
        );
    }

    /// In single mode changes coalesce, and the consumer only ever learns about the
    /// survivor. Reporting the immediate predecessor would name a value it never
    /// received, so the oldest unconsumed previous has to be carried forward.
    #[test]
    fn coalesced_changes_keep_the_previous_value_the_consumer_last_saw() {
        let manager = SignalsManager::new();
        manager.set_register(10, true, true);

        // a -> b -> c, all before anything is consumed.
        manager.set_with_previous(10, b("b"), b("a"));
        manager.set_with_previous(10, b("c"), b("b"));

        let (id, value, previous) = manager.wait_changed_value(None);
        assert_eq!(id, 10);
        assert_eq!(value, b("c"));
        assert_eq!(
            previous,
            Some(b("a")),
            "previous must be the last value delivered, not the skipped intermediate"
        );
    }

    #[test]
    fn queued_changes_each_keep_their_own_previous_value() {
        let manager = SignalsManager::new();
        manager.set_register(11, true, true);
        manager.set_to_queue(11);

        manager.set_with_previous(11, b("b"), b("a"));
        manager.set_with_previous(11, b("c"), b("b"));

        // Every queued change is delivered, so the chain is already contiguous.
        let (_, value, previous) = manager.wait_changed_value(None);
        assert_eq!((value, previous), (b("b"), Some(b("a"))));
        let (_, value, previous) = manager.wait_changed_value(Some(11));
        assert_eq!((value, previous), (b("c"), Some(b("b"))));
    }

    /// Collapsing a queue drops every change but the newest, so the survivor has to
    /// inherit the oldest previous -- same reasoning as coalescing in single mode.
    #[test]
    fn collapsing_a_queue_keeps_the_oldest_previous_value() {
        let manager = SignalsManager::new();
        manager.set_register(12, true, true);
        manager.set_to_queue(12);

        manager.set_with_previous(12, b("b"), b("a"));
        manager.set_with_previous(12, b("c"), b("b"));
        manager.set_to_single(12);

        let (_, value, previous) = manager.wait_changed_value(None);
        assert_eq!((value, previous), (b("c"), Some(b("a"))));
    }

    /// With a single queued change there is nothing older to inherit, so it has to
    /// keep its own previous -- the front and the back are the same element here.
    #[test]
    fn collapsing_a_queue_of_one_keeps_that_change_untouched() {
        let manager = SignalsManager::new();
        manager.set_register(15, true, true);
        manager.set_to_queue(15);

        manager.set_with_previous(15, b("b"), b("a"));
        manager.set_to_single(15);

        let (_, value, previous) = manager.wait_changed_value(None);
        assert_eq!((value, previous), (b("b"), Some(b("a"))));
    }

    /// An empty queue is reachable (`set_to_queue` on an id with nothing pending), and
    /// collapsing it must leave the id behaving as a plain single from then on.
    #[test]
    fn collapsing_an_empty_queue_leaves_the_id_in_single_mode() {
        let manager = SignalsManager::new();
        manager.set_register(16, true, true);
        manager.set_to_queue(16);
        manager.set_to_single(16);

        manager.set_with_previous(16, b("b"), b("a"));
        manager.set_with_previous(16, b("c"), b("b"));

        // Coalesced rather than queued, which is what single mode means.
        let (_, value, previous) = manager.wait_changed_value(None);
        assert_eq!((value, previous), (b("c"), Some(b("a"))));
        assert!(manager.values.lock().get(Some(16)).is_none());
    }

    /// The previous value is always stored, so registering for it later is not racy,
    /// but a consumer that did not ask must never be handed it -- that is what keeps
    /// the eager Python decode off the fast path.
    #[test]
    fn previous_value_is_withheld_from_consumers_that_did_not_register_for_it() {
        let manager = SignalsManager::new();
        manager.set_register(13, true, false);

        manager.set_with_previous(13, b("new"), b("old"));
        let (_, value, previous) = manager.wait_changed_value(None);
        assert_eq!(value, b("new"));
        assert_eq!(previous, None);

        // Registering for it afterwards reaches the value stored in the meantime:
        // it was kept all along, only masked on the way out.
        manager.set_register(13, true, true);
        manager.set_with_previous(13, b("newer"), b("new"));
        let (_, value, previous) = manager.wait_changed_value(Some(13));
        assert_eq!((value, previous), (b("newer"), Some(b("new"))));
    }

    /// A `Signal` stores no value, so it has nothing to report as previous.
    #[test]
    fn a_signal_reports_no_previous_value_even_when_registered_for_one() {
        let manager = SignalsManager::new();
        manager.set_register(14, true, true);

        manager.set(14, b("fired"));
        let (_, value, previous) = manager.wait_changed_value(None);
        assert_eq!(value, b("fired"));
        assert_eq!(previous, None);
    }

    fn b(value: &str) -> Bytes {
        Bytes::copy_from_slice(value.as_bytes())
    }
}
