use std::collections::{VecDeque, hash_map::Entry};
use std::sync::Arc;
#[cfg(any(feature = "server", feature = "python"))]
use std::sync::atomic::AtomicBool;
#[cfg(any(feature = "python", feature = "c_abi"))]
use std::sync::atomic::Ordering;
#[cfg(feature = "c_abi")]
use std::time::Instant;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::event::Event;
#[cfg(feature = "c_abi")]
use crate::event::WaitOutcome;
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
        // Keep claimed ids blocked: their callbacks may still be running and are
        // released when the worker passes the id back.
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

/// Result of [`SignalsManager::claim_next`].
#[cfg(feature = "c_abi")]
pub(crate) enum ClaimResult {
    /// A claimed change: id, value, and the previous value when registered for it.
    Ready(u64, Bytes, Option<Bytes>),
    /// The deadline passed without a change.
    TimedOut,
    /// [`SignalsManager::close`] was called.
    Closed,
}

#[derive(Clone)]
pub(crate) struct SignalsManager {
    event: Event,
    values: Arc<Mutex<ChangedInner>>,
    #[cfg(any(feature = "python", feature = "c_abi"))]
    closed: Arc<AtomicBool>,
}

impl SignalsManager {
    pub(crate) fn new() -> Self {
        Self {
            event: Event::new(),
            values: Arc::new(Mutex::new(ChangedInner::new())),
            #[cfg(any(feature = "python", feature = "c_abi"))]
            closed: Arc::new(AtomicBool::new(false)),
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

    /// Blocks until a registered change is available, or returns `None` once
    /// [`Self::close`] has been called.
    #[cfg(feature = "python")]
    pub(crate) fn wait_changed_value(
        &self,
        mut last_id: Option<u64>,
    ) -> Option<(u64, Bytes, Option<Bytes>)> {
        loop {
            if self.closed.load(Ordering::Acquire) {
                return None;
            }
            if let Some(val) = self.values.lock().get(last_id.take()) {
                // Pass the wake-up on, as in `claim_next`.
                self.event.set_one();
                return Some(val);
            }
            if !self.event.wait_clear_until(&self.closed) {
                return None;
            }
        }
    }

    /// Permanently wakes every waiter in [`Self::wait_changed_value`] and makes
    /// it return `None`, so the workers waiting on it can exit.
    #[cfg(any(feature = "python", feature = "c_abi"))]
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.event.set();
    }

    #[cfg(feature = "server")]
    pub(crate) fn try_changed_value(
        &self,
        last_id: Option<u64>,
    ) -> Option<(u64, Bytes, Option<Bytes>)> {
        let value = self.values.lock().get(last_id);
        if value.is_some() {
            // Pass the wake-up on, as in `claim_next`.
            self.event.set_one();
        }
        value
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
    #[cfg(any(feature = "python", feature = "c_abi"))]
    pub(crate) fn release(&self, id: u64) {
        self.values.lock().blocked_list.remove(&id);
        // Anything that arrived while the id was blocked did not set the event
        // (`ChangedInner::set` skips blocked ids), so wake a waiter to drain it.
        // A spurious wake-up is harmless: the waiter just finds nothing.
        self.event.set_one();
    }

    /// Claims the next registered change, waiting until `deadline` (`None`
    /// waits indefinitely).
    ///
    /// The returned id stays blocked for every other consumer until the caller
    /// passes it to [`Self::release`], which it must do exactly once.
    #[cfg(feature = "c_abi")]
    pub(crate) fn claim_next(&self, deadline: Option<Instant>) -> ClaimResult {
        loop {
            if self.closed.load(Ordering::Acquire) {
                return ClaimResult::Closed;
            }
            if let Some((id, value, previous)) = self.values.lock().get(None) {
                // The event is a flag and `notify(1)` does not add listeners,
                // so two changes that arrive before any waiter wakes up are
                // announced only once. Pass the wake-up on, so another waiter
                // checks for a second ready id instead of sleeping until the
                // next change.
                self.event.set_one();
                return ClaimResult::Ready(id, value, previous);
            }
            match self.event.wait_clear_deadline(&self.closed, deadline) {
                WaitOutcome::Set => {}
                WaitOutcome::Stopped => return ClaimResult::Closed,
                WaitOutcome::TimedOut => return ClaimResult::TimedOut,
            }
        }
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    #[cfg(feature = "c_abi")]
    use std::time::Instant;

    use super::*;

    /// A caller that fails before passing an id back as `last_id` must not
    /// strand it: `release` is what keeps signals queued behind it reachable.
    #[test]
    #[cfg(feature = "python")]
    fn release_frees_an_id_that_was_never_passed_back() {
        if crate::test_support::isolated() {
            return;
        }
        let manager = SignalsManager::new();
        manager.set_register(5, true, false);
        manager.set_register(9, true, false);

        manager.set(5, Bytes::from_static(b"first"));
        assert_eq!(manager.wait_changed_value(None).unwrap().0, 5);

        // A second signal for 5 queues silently -- it is blocked, so no wake-up
        // is sent and only 9 is reachable.
        manager.set(5, Bytes::from_static(b"stranded"));
        manager.set(9, Bytes::from_static(b"other"));
        assert_eq!(
            manager.wait_changed_value(None).unwrap().0,
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
            let _ = sender.send(waiter.wait_changed_value(None).unwrap().0);
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).ok(),
            Some(5),
            "the signal queued behind 5 should be reachable again"
        );
    }

    /// Python workers block here with no other exit; `close` must release every
    /// one of them, and later calls must not block.
    #[test]
    #[cfg(feature = "python")]
    fn close_releases_blocked_waiters() {
        if crate::test_support::isolated() {
            return;
        }
        let manager = SignalsManager::new();
        manager.set_register(5, true, false);

        let (sender, receiver) = mpsc::channel();
        for _ in 0..2 {
            let waiter = manager.clone();
            let sender = sender.clone();
            thread::spawn(move || {
                let _ = sender.send(waiter.wait_changed_value(None).is_none());
            });
        }
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());

        manager.close();
        for _ in 0..2 {
            assert_eq!(
                receiver.recv_timeout(Duration::from_secs(2)).ok(),
                Some(true)
            );
        }
        manager.set(5, b("pending"));
        assert!(manager.wait_changed_value(None).is_none());
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

        let (id, value, previous) = claim(&manager, None);
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
        let (_, value, previous) = claim(&manager, None);
        assert_eq!((value, previous), (b("b"), Some(b("a"))));
        let (_, value, previous) = claim(&manager, Some(11));
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

        let (_, value, previous) = claim(&manager, None);
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

        let (_, value, previous) = claim(&manager, None);
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
        let (_, value, previous) = claim(&manager, None);
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
        let (_, value, previous) = claim(&manager, None);
        assert_eq!(value, b("new"));
        assert_eq!(previous, None);

        // Registering for it afterwards reaches the value stored in the meantime:
        // it was kept all along, only masked on the way out.
        manager.set_register(13, true, true);
        manager.set_with_previous(13, b("newer"), b("new"));
        let (_, value, previous) = claim(&manager, Some(13));
        assert_eq!((value, previous), (b("newer"), Some(b("new"))));
    }

    /// A `Signal` stores no value, so it has nothing to report as previous.
    #[test]
    fn a_signal_reports_no_previous_value_even_when_registered_for_one() {
        let manager = SignalsManager::new();
        manager.set_register(14, true, true);

        manager.set(14, b("fired"));
        let (_, value, previous) = claim(&manager, None);
        assert_eq!(value, b("fired"));
        assert_eq!(previous, None);
    }

    /// A claimed id stays with its worker until passed back as `last_id`, which is
    /// what keeps the callbacks of one state from running concurrently.
    #[test]
    fn a_claimed_id_is_not_handed_out_again_until_it_is_passed_back() {
        let manager = SignalsManager::new();
        manager.set_register(20, true, false);

        manager.set(20, b("first"));
        assert_eq!(claim(&manager, None).0, 20);
        manager.set(20, b("second"));
        assert!(
            manager.values.lock().get(None).is_none(),
            "another worker claimed an id that is still running"
        );
        let (id, value, _) = claim(&manager, Some(20));
        assert_eq!((id, value), (20, b("second")));
    }

    /// A disconnect resets the pending changes, but a worker may still be running
    /// the callback it claimed before. Unblocking that id lets a second worker run
    /// the same state's callbacks concurrently.
    #[test]
    fn reset_keeps_a_claimed_id_blocked() {
        let manager = SignalsManager::new();
        manager.set_register(21, true, false);

        manager.set(21, b("before disconnect"));
        assert_eq!(claim(&manager, None).0, 21);
        manager.reset();
        manager.set(21, b("after reconnect"));
        assert!(
            manager.values.lock().get(None).is_none(),
            "reset released an id whose callback is still running"
        );
        let (id, value, _) = claim(&manager, Some(21));
        assert_eq!((id, value), (21, b("after reconnect")));
    }

    /// Pins current behavior: a disconnect discards the client's pending changes
    /// that no callback has claimed yet, but keeps the server's own events.
    #[test]
    fn reset_discards_pending_client_changes_and_keeps_system_events() {
        let manager = SignalsManager::new();
        manager.set_register(22, true, false);
        manager.set_register(LOGGING_ID, true, false);
        manager.set_register(ON_DISCONNECT_ID, true, false);

        manager.set(22, b("pending"));
        manager.error("logged");
        manager.on_disconnect();
        manager.reset();

        let mut claimed = Vec::new();
        let mut last = None;
        while let Some((id, _, _)) = manager.values.lock().get(last) {
            claimed.push(id);
            last = Some(id);
        }
        assert_eq!(claimed, [LOGGING_ID, ON_DISCONNECT_ID]);
    }

    #[cfg(feature = "c_abi")]
    fn claim_ready(manager: &SignalsManager, deadline: Option<Instant>) -> (u64, Bytes) {
        match manager.claim_next(deadline) {
            ClaimResult::Ready(id, value, _) => (id, value),
            ClaimResult::TimedOut => panic!("claim timed out"),
            ClaimResult::Closed => panic!("signals closed"),
        }
    }

    #[test]
    #[cfg(feature = "c_abi")]
    fn claim_next_times_out_without_changes() {
        let manager = SignalsManager::new();
        manager.set_register(30, true, false);

        let start = Instant::now();
        let result = manager.claim_next(Some(start + Duration::from_millis(50)));
        assert!(matches!(result, ClaimResult::TimedOut));
        assert!(start.elapsed() >= Duration::from_millis(50));
        assert!(start.elapsed() < Duration::from_secs(5));

        // A past deadline still claims a change that is already pending.
        manager.set(30, b("ready"));
        let (id, value) = claim_ready(&manager, Some(Instant::now()));
        assert_eq!((id, value), (30, b("ready")));
    }

    /// A claim taken with `claim_next` is held until `release`, so a second
    /// change for the same state cannot be handed to another consumer.
    #[test]
    #[cfg(feature = "c_abi")]
    fn claim_next_holds_the_id_until_release() {
        let manager = SignalsManager::new();
        manager.set_register(31, true, false);

        manager.set(31, b("first"));
        assert_eq!(claim_ready(&manager, None).0, 31);
        manager.set(31, b("second"));
        assert!(matches!(
            manager.claim_next(Some(Instant::now())),
            ClaimResult::TimedOut
        ));

        manager.release(31);
        assert_eq!(
            claim_ready(&manager, Some(Instant::now())),
            (31, b("second"))
        );
    }

    #[test]
    #[cfg(feature = "c_abi")]
    fn close_wakes_a_blocked_claim() {
        if crate::test_support::isolated() {
            return;
        }
        let manager = SignalsManager::new();
        manager.set_register(32, true, false);

        let (sender, receiver) = mpsc::channel();
        let waiter = manager.clone();
        thread::spawn(move || {
            let closed = matches!(waiter.claim_next(None), ClaimResult::Closed);
            let _ = sender.send(closed);
        });
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());

        manager.close();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).ok(),
            Some(true)
        );
        manager.set(32, b("late"));
        assert!(matches!(manager.claim_next(None), ClaimResult::Closed));
    }

    /// Two changes that arrive before any waiter wakes notify only once; the
    /// first waiter must pass the wake-up on so the second id is not stranded.
    #[test]
    #[cfg(feature = "c_abi")]
    fn every_waiter_receives_a_ready_id() {
        if crate::test_support::isolated() {
            return;
        }
        let manager = SignalsManager::new();
        manager.set_register(33, true, false);
        manager.set_register(34, true, false);

        let (sender, receiver) = mpsc::channel();
        for _ in 0..2 {
            let waiter = manager.clone();
            let sender = sender.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                if let ClaimResult::Ready(id, _, _) = waiter.claim_next(Some(deadline)) {
                    let _ = sender.send(id);
                }
            });
        }
        thread::sleep(Duration::from_millis(50));
        manager.set(33, b("a"));
        manager.set(34, b("b"));

        let mut ids = [
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        ];
        ids.sort_unstable();
        assert_eq!(ids, [33, 34]);
    }

    /// Two changes that arrive before any worker wakes notify only once. The
    /// worker that claims the first must pass the wake-up on, or the second
    /// waits until the first callback has finished.
    fn every_worker_receives_a_ready_id(claim_one: fn(SignalsManager) -> Option<u64>) {
        let manager = SignalsManager::new();
        manager.set_register(35, true, false);
        manager.set_register(36, true, false);

        let (sender, receiver) = mpsc::channel();
        for _ in 0..2 {
            let worker = manager.clone();
            let sender = sender.clone();
            // Each worker keeps its claim, like one that is still running a callback.
            thread::spawn(move || {
                if let Some(id) = claim_one(worker) {
                    let _ = sender.send(id);
                }
            });
        }
        thread::sleep(Duration::from_millis(50));
        manager.set(35, b("a"));
        manager.set(36, b("b"));

        let mut ids = [
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        ];
        ids.sort_unstable();
        assert_eq!(ids, [35, 36]);
    }

    /// The claim loop of the Rust callback worker.
    #[test]
    #[cfg(feature = "server")]
    fn every_polling_worker_receives_a_ready_id() {
        if crate::test_support::isolated() {
            return;
        }
        every_worker_receives_a_ready_id(|worker| {
            let stop = AtomicBool::new(false);
            loop {
                if let Some((id, _, _)) = worker.try_changed_value(None) {
                    return Some(id);
                }
                if !worker.wait_for_change_until(&stop) {
                    return None;
                }
            }
        });
    }

    /// The claim loop of the Python callback worker.
    #[test]
    #[cfg(feature = "python")]
    fn every_blocking_worker_receives_a_ready_id() {
        if crate::test_support::isolated() {
            return;
        }
        every_worker_receives_a_ready_id(|worker| {
            worker.wait_changed_value(None).map(|(id, _, _)| id)
        });
    }

    fn claim(manager: &SignalsManager, last: Option<u64>) -> (u64, Bytes, Option<Bytes>) {
        manager
            .values
            .lock()
            .get(last)
            .expect("expected a pending change")
    }

    fn b(value: &str) -> Bytes {
        Bytes::copy_from_slice(value.as_bytes())
    }
}
