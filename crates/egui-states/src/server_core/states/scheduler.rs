//! Transfer scheduling shared by the states whose transfers the client
//! acknowledges: `Data`, `DataMulti`, `Image` and `ImageMulti`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::lock_api::ArcMutexGuard;
use parking_lot::{Mutex, RawMutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::event::Event;
use crate::hashing::NoHashMap;
use crate::protocol::FastVec;
use crate::server_core::net::sender::MessageSender;
use crate::server_core::transfer::Messages;

/// Connection bookkeeping a state keeps under its storage lock.
///
/// A producer stores its change and, under the same lock, asks whether it may
/// send it. Until the state is synchronized with a newly connected client it
/// may not: the synchronization sends the stored data, so a delta sent as well
/// would be applied twice. A reset bumps the generation, so a producer that
/// was waiting for an acknowledgement of a previous connection drops its work.
pub(crate) struct SyncState {
    required: bool,
    generation: u64,
}

impl SyncState {
    pub(crate) fn new() -> Self {
        Self {
            required: true,
            generation: 0,
        }
    }

    /// The generation to send in, or `None` if the change must only be stored.
    pub(crate) fn sendable(&self, connected: &AtomicBool) -> Option<u64> {
        (!self.required && connected.load(Ordering::Acquire)).then_some(self.generation)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the state awaits a synchronization with a newly connected
    /// client.
    pub(crate) fn required(&self) -> bool {
        self.required
    }

    pub(crate) fn synchronized(&mut self) {
        self.required = false;
    }

    pub(crate) fn reset(&mut self) {
        self.required = true;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Whether the rectangle `outer` (`[y, x, height, width]`) contains `inner`.
fn covers(outer: [usize; 4], inner: [usize; 4]) -> bool {
    outer[0] <= inner[0]
        && outer[1] <= inner[1]
        && outer[0] + outer[2] >= inner[0] + inner[2]
        && outer[1] + outer[3] >= inner[1] + inner[3]
}

/// The parts of a split update that wait behind the part in flight.
struct Waiting {
    /// The rectangle (`[y, x, height, width]`) of the whole update.
    rect: [usize; 4],
    parts: VecDeque<Messages>,
}

struct State {
    sync: SyncState,
    /// The waiting parts of a split update, by key.
    waiting: NoHashMap<u32, Waiting>,
    /// The keys whose synchronization transfer the client has not
    /// acknowledged yet.
    syncing: NoHashMap<u32, ()>,
}

/// The transfer of one key.
struct Slot {
    /// Serializes the producers of the key; held while they wait.
    producer: Arc<Mutex<()>>,
    /// Set while nothing of the key is in flight and nothing waits.
    event: Event,
}

impl Slot {
    fn new() -> Self {
        let event = Event::new();
        event.set();
        Self {
            producer: Arc::new(Mutex::new(())),
            event,
        }
    }
}

/// A producer's hold on one key: no other producer of the key runs, and no
/// operation on every key starts, until it is dropped.
pub(super) struct KeyLock<'a> {
    key: u32,
    slot: Arc<Slot>,
    _producer: ArcMutexGuard<RawMutex, ()>,
    _gate: RwLockReadGuard<'a, ()>,
}

/// A hold on every key: no producer runs until it is dropped.
pub(super) struct AllLock<'a> {
    _gate: RwLockWriteGuard<'a, ()>,
}

/// The keys a lock holds: one, or every key (`None`).
pub(super) trait Held {
    fn key(&self) -> Option<u32>;
}

impl Held for KeyLock<'_> {
    fn key(&self) -> Option<u32> {
        Some(self.key)
    }
}

impl Held for AllLock<'_> {
    fn key(&self) -> Option<u32> {
        None
    }
}

/// Schedules the acknowledged transfers of one state on its connection.
///
/// Each key has its own transfer: one is in flight at a time per key, and the
/// keys do not wait for each other. A state with a single value uses one key.
/// A producer waits until its key is idle, with nothing in flight and nothing
/// waiting, and then sends. Only a split update leaves work behind: its first
/// part is sent and the others wait, each sent when the client acknowledged
/// the one before. A set of the same key drops those waiting parts, because it
/// overwrites all of them, and so does a forced update whose rectangle covers
/// theirs.
///
/// Producers of a key are serialized by its [`KeyLock`], which they hold while
/// they wait, so the changes of one key reach the client in the order they
/// were made; changes of different keys can overtake each other. An operation
/// on every key (a reset of a keyed state, a synchronization) holds an
/// [`AllLock`], so it waits until no producer is between storing a change and
/// sending it. All messages of a transfer are queued under the state lock, so
/// the transfers of different keys never interleave on the wire.
pub(super) struct Scheduler {
    pub(super) id: u64,
    /// Held shared by [`KeyLock`] and exclusively by [`AllLock`].
    gate: RwLock<()>,
    slots: Mutex<NoHashMap<u32, Arc<Slot>>>,
    state: Mutex<State>,
    pub(super) sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl Scheduler {
    pub(super) fn new(id: u64, sender: MessageSender, connected: Arc<AtomicBool>) -> Self {
        Self {
            id,
            gate: RwLock::new(()),
            slots: Mutex::new(NoHashMap::default()),
            state: Mutex::new(State {
                sync: SyncState::new(),
                waiting: NoHashMap::default(),
                syncing: NoHashMap::default(),
            }),
            sender,
            connected,
        }
    }

    #[inline]
    pub(super) fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// The generation to send in, or `None` while the state awaits a
    /// synchronization or no client is connected. Producers ask while they
    /// hold the state's storage lock, so a reset after their check
    /// invalidates their work.
    pub(super) fn sendable(&self) -> Option<u64> {
        self.state.lock().sync.sendable(&self.connected)
    }

    pub(super) fn required(&self) -> bool {
        self.state.lock().sync.required()
    }

    #[cfg(test)]
    pub(super) fn generation(&self) -> u64 {
        self.state.lock().sync.generation()
    }

    /// Whether parts of a split update wait behind a part in flight.
    #[cfg(all(test, feature = "client"))]
    pub(super) fn has_waiting(&self) -> bool {
        !self.state.lock().waiting.is_empty()
    }

    /// Whether a producer of `key` holds its lock.
    #[cfg(test)]
    pub(super) fn is_locked(&self, key: u32) -> bool {
        self.slot(key).producer.try_lock().is_none()
    }

    fn slot(&self, key: u32) -> Arc<Slot> {
        self.slots
            .lock()
            .entry(key)
            .or_insert_with(|| Arc::new(Slot::new()))
            .clone()
    }

    /// Waits until no other producer of `key` runs and no operation on every
    /// key does, and holds the key until the lock is dropped.
    pub(super) fn lock(&self, key: u32) -> KeyLock<'_> {
        let gate = self.gate.read();
        let slot = self.slot(key);
        let producer = slot.producer.lock_arc();
        KeyLock {
            key,
            slot,
            _producer: producer,
            _gate: gate,
        }
    }

    /// Waits until no producer runs, and holds every key until the lock is
    /// dropped.
    pub(super) fn lock_all(&self) -> AllLock<'_> {
        AllLock {
            _gate: self.gate.write(),
        }
    }

    /// Whether nothing is in flight and nothing waits, for every key, so a
    /// change would be sent at once. Also true while no client is
    /// synchronized, because a change is then only stored.
    pub(super) fn is_idle(&self) -> bool {
        self.sendable().is_none() || self.slots.lock().values().all(|slot| slot.event.is_set())
    }

    /// Whether nothing of `key` is in flight and nothing of it waits, as
    /// [`is_idle`](Self::is_idle) for one key.
    pub(super) fn is_key_idle(&self, key: u32) -> bool {
        self.sendable().is_none()
            || self
                .slots
                .lock()
                .get(&key)
                .is_none_or(|slot| slot.event.is_set())
    }

    /// Sends a set of the locked key once the key is idle. Its waiting parts
    /// are dropped first: the set overwrites them.
    pub(super) fn send_set(
        &self,
        lock: KeyLock<'_>,
        messages: Messages,
        wait: bool,
        generation: u64,
    ) {
        {
            let mut state = self.state.lock();
            if state.sync.generation() != generation {
                return;
            }
            state.waiting.remove(&lock.key);
        }
        self.send(lock, [0; 4], VecDeque::from([messages]), wait, generation);
    }

    /// Sends the parts of an update of the locked key once the key is idle.
    /// With `force`, its waiting parts are dropped first when `rect` covers
    /// their rectangle.
    pub(super) fn send_update(
        &self,
        lock: KeyLock<'_>,
        rect: [usize; 4],
        parts: VecDeque<Messages>,
        force: bool,
        wait: bool,
        generation: u64,
    ) {
        {
            let mut state = self.state.lock();
            if state.sync.generation() != generation {
                return;
            }
            if force
                && state
                    .waiting
                    .get(&lock.key)
                    .is_some_and(|waiting| covers(rect, waiting.rect))
            {
                state.waiting.remove(&lock.key);
            }
        }
        self.send(lock, rect, parts, wait, generation);
    }

    /// Waits until the key is idle, sends the first part and leaves the
    /// others waiting. With `wait`, also waits until the client acknowledged
    /// the last part.
    fn send(
        &self,
        lock: KeyLock<'_>,
        rect: [usize; 4],
        mut parts: VecDeque<Messages>,
        wait: bool,
        generation: u64,
    ) {
        lock.slot.event.wait_clear();
        // Keep reset from changing the generation between this check and the
        // enqueue, which could otherwise leak stale work into a new
        // connection. Stale work leaves the event clear: the next
        // synchronization sets it again.
        let mut state = self.state.lock();
        if state.sync.generation() != generation || !self.is_connected() {
            return;
        }
        let Some(first) = parts.pop_front() else {
            lock.slot.event.set();
            return;
        };
        if !parts.is_empty() {
            state.waiting.insert(lock.key, Waiting { rect, parts });
        }
        self.send_all(first);
        drop(state);

        // The key's lock is still held, so only this transfer's
        // acknowledgements or a disconnect set the event.
        if wait {
            lock.slot.event.wait();
        }
    }

    /// Drops the waiting parts of the locked key, then queues `message`,
    /// which the client does not acknowledge, once the key is idle.
    pub(super) fn send_control(&self, lock: KeyLock<'_>, message: FastVec<32>, generation: u64) {
        self.state.lock().waiting.remove(&lock.key);
        lock.slot.event.wait();
        let state = self.state.lock();
        if state.sync.generation() != generation || !self.is_connected() {
            return;
        }
        self.sender.send(message);
    }

    /// Drops the waiting parts of every key, then queues `message`, which the
    /// client does not acknowledge, once every key is idle. The keys are
    /// forgotten, as after a reset of the state.
    pub(super) fn send_control_all(
        &self,
        lock: AllLock<'_>,
        message: FastVec<32>,
        generation: u64,
    ) {
        self.state.lock().waiting.clear();
        let slots = self.slots.lock().values().cloned().collect::<Vec<_>>();
        for slot in slots {
            slot.event.wait();
        }
        {
            let state = self.state.lock();
            if state.sync.generation() == generation && self.is_connected() {
                self.sender.send(message);
            }
        }
        self.forget_idle_keys(&lock);
    }

    /// Queues `message`, which the client does not acknowledge, at once: it
    /// follows the transfers of earlier producers of the locked keys on the
    /// wire without waiting for their acknowledgement. Only for states that
    /// never leave split parts waiting, which it would overtake; others use
    /// [`send_control`](Self::send_control).
    pub(super) fn send_unacked(&self, lock: impl Held, message: FastVec<32>, generation: u64) {
        let state = self.state.lock();
        debug_assert!(
            match lock.key() {
                Some(key) => !state.waiting.contains_key(&key),
                None => state.waiting.is_empty(),
            },
            "would overtake waiting parts"
        );
        if state.sync.generation() != generation || !self.is_connected() {
            return;
        }
        self.sender.send(message);
        drop(state);
        if lock.key().is_none() {
            self.forget_idle_keys(&lock);
        }
    }

    /// Drops the slots of idle keys that no producer holds, so a state whose
    /// keys change does not keep a slot for every key it ever had.
    fn forget_idle_keys(&self, _lock: &impl Held) {
        self.slots
            .lock()
            .retain(|_, slot| Arc::strong_count(slot) > 1 || !slot.event.is_set());
    }

    /// Starts a connection synchronization: sends `messages`, and makes each
    /// of `keys` wait for the acknowledgement of its synchronization
    /// transfer. The caller queues the transfers of `keys` that are not in
    /// `messages` itself, with [`send_synced`](Self::send_synced).
    pub(super) fn begin_sync(&self, _lock: &AllLock<'_>, messages: Messages, keys: &[u32]) {
        let mut state = self.state.lock();
        state.sync.synchronized();
        state.waiting.clear();
        state.syncing.clear();
        // No producer runs: the events stale work left clear are set again.
        for slot in self.slots.lock().values() {
            slot.event.set();
        }
        for &key in keys {
            state.syncing.insert(key, ());
            self.slot(key).event.clear();
        }
        self.send_all(messages);
    }

    /// Queues a synchronization transfer of a key passed to
    /// [`begin_sync`](Self::begin_sync).
    pub(super) fn send_synced(&self, _lock: &AllLock<'_>, messages: Messages) {
        self.send_all(messages);
    }

    pub(super) fn acknowledge(&self, key: u32) {
        let mut state = self.state.lock();
        let next = if state.syncing.remove(&key).is_some() {
            None
        } else {
            let next = state
                .waiting
                .get_mut(&key)
                .and_then(|waiting| waiting.parts.pop_front());
            if state
                .waiting
                .get(&key)
                .is_some_and(|waiting| waiting.parts.is_empty())
            {
                state.waiting.remove(&key);
            }
            next
        };
        match next {
            Some(part) => self.send_all(part),
            // An acknowledgement of a key without a slot belongs to a key
            // that was forgotten; nothing waits for it.
            None => {
                if let Some(slot) = self.slots.lock().get(&key) {
                    slot.event.set();
                }
            }
        }
    }

    pub(super) fn reset(&self) {
        // Bump the generation before releasing waiting producers, so they see
        // that their work belongs to the old connection.
        {
            let mut state = self.state.lock();
            state.waiting.clear();
            state.syncing.clear();
            state.sync.reset();
        }
        for slot in self.slots.lock().values() {
            slot.event.set();
        }
    }

    fn send_all(&self, messages: Messages) {
        for (message, send_now) in messages {
            self.sender.send_set(message, send_now);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
    use std::time::Duration;

    use tokio::sync::mpsc::error::TryRecvError;

    use super::*;
    use crate::server_core::net::sender::MessageReceiver;

    const RECT: [usize; 4] = [0, 0, 4, 4];

    fn scheduler() -> (Arc<Scheduler>, Arc<AtomicBool>, MessageReceiver) {
        let connected = Arc::new(AtomicBool::new(true));
        let (sender, receiver) = MessageSender::new();
        let scheduler = Arc::new(Scheduler::new(1, sender, connected.clone()));
        scheduler.begin_sync(&scheduler.lock_all(), Vec::new(), &[]);
        (scheduler, connected, receiver)
    }

    fn marker(marker: u8) -> FastVec<32> {
        let mut message = FastVec::new();
        message.extend_from_slice(&[marker]);
        message
    }

    fn part(value: u8) -> Messages {
        vec![(marker(value), false)]
    }

    fn assert_marker(receiver: &mut MessageReceiver, expected: u8) {
        let (message, _) = receiver.try_recv().unwrap().unwrap();
        assert_eq!(message.into_bytes().as_ref(), &[expected]);
    }

    fn assert_no_message(receiver: &mut MessageReceiver) {
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    fn set(scheduler: &Scheduler, key: u32, value: u8, wait: bool) {
        let generation = scheduler.generation();
        scheduler.send_set(scheduler.lock(key), part(value), wait, generation);
    }

    fn update(
        scheduler: &Scheduler,
        key: u32,
        rect: [usize; 4],
        values: &[u8],
        force: bool,
        wait: bool,
    ) {
        let generation = scheduler.generation();
        let parts = values.iter().map(|&value| part(value)).collect();
        scheduler.send_update(scheduler.lock(key), rect, parts, force, wait, generation);
    }

    fn control(scheduler: &Scheduler, key: Option<u32>, value: u8) {
        let generation = scheduler.generation();
        match key {
            Some(key) => scheduler.send_control(scheduler.lock(key), marker(value), generation),
            None => scheduler.send_control_all(scheduler.lock_all(), marker(value), generation),
        }
    }

    /// Runs `operation` on another thread and checks that it blocks.
    fn spawn_blocked(
        scheduler: &Arc<Scheduler>,
        operation: impl FnOnce(&Scheduler) + Send + 'static,
    ) -> Receiver<()> {
        let scheduler = scheduler.clone();
        let (done_sender, done) = channel();
        std::thread::spawn(move || {
            operation(&scheduler);
            done_sender.send(()).unwrap();
        });
        assert_blocked(&done);
        done
    }

    fn assert_blocked(done: &Receiver<()>) {
        assert!(matches!(
            done.recv_timeout(Duration::from_millis(100)),
            Err(RecvTimeoutError::Timeout)
        ));
    }

    fn finished(done: &Receiver<()>) {
        done.recv_timeout(Duration::from_secs(5))
            .expect("blocked producer did not finish");
    }

    #[test]
    fn is_idle_while_no_client_is_synchronized() {
        let connected = Arc::new(AtomicBool::new(false));
        let (sender, mut receiver) = MessageSender::new();
        let scheduler = Scheduler::new(1, sender, connected.clone());
        assert!(scheduler.is_idle());
        connected.store(true, Ordering::Release);
        assert!(
            scheduler.is_idle(),
            "a change before the sync is only stored"
        );

        scheduler.begin_sync(&scheduler.lock_all(), part(1), &[0]);
        assert_marker(&mut receiver, 1);
        assert!(!scheduler.is_idle());
        assert!(!scheduler.is_key_idle(0));
        assert!(scheduler.is_key_idle(1), "a key without a transfer is idle");
        scheduler.acknowledge(0);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn a_set_waits_for_the_acknowledgement_of_the_transfer_in_flight() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        set(&scheduler, 7, 1, false);
        assert_marker(&mut receiver, 1);
        assert!(!scheduler.is_idle());

        let done = spawn_blocked(&scheduler, |s| set(s, 7, 2, false));
        assert_no_message(&mut receiver);
        scheduler.acknowledge(7);
        finished(&done);
        assert_marker(&mut receiver, 2);
        scheduler.acknowledge(7);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn the_keys_do_not_wait_for_each_other() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        set(&scheduler, 7, 1, false);
        set(&scheduler, 9, 2, false);
        assert_marker(&mut receiver, 1);
        assert_marker(&mut receiver, 2);

        // An acknowledgement releases its own key only.
        scheduler.acknowledge(9);
        assert!(scheduler.is_key_idle(9));
        assert!(!scheduler.is_key_idle(7));
        assert!(!scheduler.is_idle());
        let done = spawn_blocked(&scheduler, |s| set(s, 7, 3, false));
        set(&scheduler, 9, 4, false);
        assert_marker(&mut receiver, 4);
        scheduler.acknowledge(9);
        assert_blocked(&done);
        scheduler.acknowledge(7);
        finished(&done);
        assert_marker(&mut receiver, 3);
        scheduler.acknowledge(7);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn a_split_update_returns_after_its_first_part_and_the_next_call_waits_for_its_last() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        update(&scheduler, 7, RECT, &[1, 2, 3], false, false);
        assert_marker(&mut receiver, 1);
        assert_no_message(&mut receiver);

        let done = spawn_blocked(&scheduler, |s| update(s, 7, RECT, &[4], false, false));
        for value in [2, 3] {
            scheduler.acknowledge(7);
            assert_marker(&mut receiver, value);
            assert_blocked(&done);
        }
        scheduler.acknowledge(7);
        finished(&done);
        assert_marker(&mut receiver, 4);
        scheduler.acknowledge(7);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn a_set_drops_the_waiting_parts_of_its_own_key_only() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        update(&scheduler, 7, RECT, &[1, 2, 3], false, false);
        assert_marker(&mut receiver, 1);
        // It still waits for the part in flight.
        let done = spawn_blocked(&scheduler, |s| set(s, 7, 4, false));
        scheduler.acknowledge(7);
        finished(&done);
        assert_marker(&mut receiver, 4);
        assert_no_message(&mut receiver);
        scheduler.acknowledge(7);
        assert!(scheduler.is_idle());

        // A set of another key is sent at once and keeps the parts.
        update(&scheduler, 7, RECT, &[5, 6], false, false);
        assert_marker(&mut receiver, 5);
        set(&scheduler, 9, 7, false);
        assert_marker(&mut receiver, 7);
        scheduler.acknowledge(7);
        assert_marker(&mut receiver, 6);
        scheduler.acknowledge(7);
        scheduler.acknowledge(9);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn a_forced_update_drops_the_waiting_parts_it_covers_of_its_own_key() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        update(&scheduler, 7, [1, 1, 2, 2], &[1, 2, 3], false, false);
        assert_marker(&mut receiver, 1);
        let done = spawn_blocked(&scheduler, |s| update(s, 7, RECT, &[4, 5], true, false));
        scheduler.acknowledge(7);
        finished(&done);
        for value in [4, 5] {
            assert_marker(&mut receiver, value);
            assert_no_message(&mut receiver);
            scheduler.acknowledge(7);
        }
        assert!(scheduler.is_idle());

        // A rectangle that does not cover: every part is sent.
        update(&scheduler, 7, [0, 0, 2, 2], &[6, 7], false, false);
        assert_marker(&mut receiver, 6);
        let done = spawn_blocked(&scheduler, |s| {
            update(s, 7, [0, 0, 1, 1], &[8], true, false)
        });
        scheduler.acknowledge(7);
        assert_marker(&mut receiver, 7);
        assert_blocked(&done);
        scheduler.acknowledge(7);
        finished(&done);
        assert_marker(&mut receiver, 8);
        scheduler.acknowledge(7);

        // Another key: sent at once, and the parts of the first stay.
        update(&scheduler, 7, [0, 0, 2, 2], &[9, 10], false, false);
        assert_marker(&mut receiver, 9);
        update(&scheduler, 9, RECT, &[11], true, false);
        assert_marker(&mut receiver, 11);
        scheduler.acknowledge(7);
        assert_marker(&mut receiver, 10);
        scheduler.acknowledge(7);
        scheduler.acknowledge(9);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn a_control_drops_waiting_parts_and_leaves_the_state_idle() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        // Removing the key drops its parts and waits for the one in flight.
        update(&scheduler, 7, RECT, &[1, 2, 3], false, false);
        assert_marker(&mut receiver, 1);
        let done = spawn_blocked(&scheduler, |s| control(s, Some(7), 10));
        scheduler.acknowledge(7);
        finished(&done);
        assert_marker(&mut receiver, 10);
        assert_no_message(&mut receiver);
        assert!(scheduler.is_idle());

        // Removing another key neither waits nor drops the parts.
        update(&scheduler, 7, RECT, &[2, 3], false, false);
        assert_marker(&mut receiver, 2);
        control(&scheduler, Some(9), 11);
        assert_marker(&mut receiver, 11);
        scheduler.acknowledge(7);
        assert_marker(&mut receiver, 3);
        scheduler.acknowledge(7);

        // A control of every key drops every part and waits for each key.
        update(&scheduler, 7, RECT, &[4, 5], false, false);
        set(&scheduler, 9, 6, false);
        assert_marker(&mut receiver, 4);
        assert_marker(&mut receiver, 6);
        let done = spawn_blocked(&scheduler, |s| control(s, None, 12));
        scheduler.acknowledge(7);
        assert_blocked(&done);
        scheduler.acknowledge(9);
        finished(&done);
        assert_marker(&mut receiver, 12);
        assert_no_message(&mut receiver);

        // Unacknowledged controls leave the state idle.
        assert!(scheduler.is_idle());
        set(&scheduler, 9, 13, false);
        assert_marker(&mut receiver, 13);
    }

    /// A control of every key waits for producers that stored a change and
    /// wait to send it, so it cannot drop a change they send afterwards.
    #[test]
    fn an_operation_on_every_key_waits_for_the_producers() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        set(&scheduler, 7, 1, false);
        assert_marker(&mut receiver, 1);
        let producer = spawn_blocked(&scheduler, |s| set(s, 7, 2, false));
        let reset = spawn_blocked(&scheduler, |s| control(s, None, 3));
        scheduler.acknowledge(7);
        finished(&producer);
        assert_marker(&mut receiver, 2);
        assert_blocked(&reset);
        scheduler.acknowledge(7);
        finished(&reset);
        assert_marker(&mut receiver, 3);
    }

    #[test]
    fn a_synchronization_waits_for_the_acknowledgement_of_each_key() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, connected, mut receiver) = scheduler();
        connected.store(false, Ordering::Release);
        scheduler.reset();
        connected.store(true, Ordering::Release);
        let mut messages = part(1);
        messages.extend(part(2));
        scheduler.begin_sync(&scheduler.lock_all(), messages, &[1, 2]);
        assert_marker(&mut receiver, 1);
        assert_marker(&mut receiver, 2);

        // A key the synchronization did not send is idle.
        set(&scheduler, 3, 3, false);
        assert_marker(&mut receiver, 3);
        let done = spawn_blocked(&scheduler, |s| set(s, 1, 4, false));
        scheduler.acknowledge(2);
        assert_blocked(&done);
        scheduler.acknowledge(1);
        finished(&done);
        assert_marker(&mut receiver, 4);
    }

    #[test]
    fn a_control_of_every_key_forgets_the_idle_keys() {
        let (scheduler, _, mut receiver) = scheduler();
        set(&scheduler, 7, 1, false);
        set(&scheduler, 9, 2, false);
        scheduler.acknowledge(7);
        scheduler.acknowledge(9);
        assert_eq!(scheduler.slots.lock().len(), 2);
        control(&scheduler, None, 3);
        assert!(scheduler.slots.lock().is_empty());
        for value in [1, 2, 3] {
            assert_marker(&mut receiver, value);
        }
        // An acknowledgement of a forgotten key is ignored.
        scheduler.acknowledge(7);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn an_unacknowledged_message_is_queued_behind_the_transfer_in_flight() {
        let (scheduler, _, mut receiver) = scheduler();
        set(&scheduler, 7, 1, false);
        assert_marker(&mut receiver, 1);

        let generation = scheduler.generation();
        scheduler.send_unacked(scheduler.lock(7), marker(2), generation);
        assert_marker(&mut receiver, 2);
        assert!(!scheduler.is_idle(), "the set is still in flight");
        scheduler.acknowledge(7);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn with_wait_a_producer_returns_once_its_last_part_is_acknowledged() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, _, mut receiver) = scheduler();
        let done = spawn_blocked(&scheduler, |s| set(s, 7, 1, true));
        assert_marker(&mut receiver, 1);
        scheduler.acknowledge(7);
        finished(&done);
        assert!(scheduler.is_idle());

        let done = spawn_blocked(&scheduler, |s| update(s, 7, RECT, &[2, 3], false, true));
        assert_marker(&mut receiver, 2);
        scheduler.acknowledge(7);
        assert_marker(&mut receiver, 3);
        assert_blocked(&done);
        // Another key's acknowledgement does not release it.
        scheduler.acknowledge(9);
        assert_blocked(&done);
        scheduler.acknowledge(7);
        finished(&done);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn a_disconnect_releases_waiting_producers_and_drops_their_work() {
        if crate::test_support::isolated() {
            return;
        }
        let (scheduler, connected, mut receiver) = scheduler();
        set(&scheduler, 7, 1, false);
        assert_marker(&mut receiver, 1);
        let done = spawn_blocked(&scheduler, |s| set(s, 7, 2, false));
        connected.store(false, Ordering::Release);
        scheduler.reset();
        finished(&done);
        assert_no_message(&mut receiver);
        assert!(scheduler.is_idle());

        connected.store(true, Ordering::Release);
        scheduler.begin_sync(&scheduler.lock_all(), Vec::new(), &[]);
        assert!(
            scheduler.is_key_idle(7),
            "the synchronization frees the key"
        );
        let done = spawn_blocked(&scheduler, |s| set(s, 7, 3, true));
        assert_marker(&mut receiver, 3);
        connected.store(false, Ordering::Release);
        scheduler.reset();
        finished(&done);
    }

    /// A producer that checked whether it may send before a reset must not
    /// send its work into the next connection, nor consume the idle state.
    #[test]
    fn work_checked_before_a_reset_is_never_sent() {
        let (scheduler, _, mut receiver) = scheduler();
        let stale = scheduler.generation();
        scheduler.reset();
        scheduler.begin_sync(&scheduler.lock_all(), Vec::new(), &[]);

        scheduler.send_set(scheduler.lock(7), part(1), false, stale);
        let parts = VecDeque::from([part(2), part(3)]);
        scheduler.send_update(scheduler.lock(7), RECT, parts, false, false, stale);
        scheduler.send_control(scheduler.lock(7), marker(4), stale);
        scheduler.send_control_all(scheduler.lock_all(), marker(4), stale);
        scheduler.send_unacked(scheduler.lock(7), marker(6), stale);
        assert_no_message(&mut receiver);
        assert!(scheduler.is_idle(), "stale work consumed the idle state");

        set(&scheduler, 7, 5, false);
        assert_marker(&mut receiver, 5);
    }
}
