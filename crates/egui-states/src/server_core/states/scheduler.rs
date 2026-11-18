//! Transfer scheduling shared by the states whose transfers the client
//! acknowledges: `Data`, `DataMulti`, `Image` and `ImageMulti`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, MutexGuard};

use crate::event::Event;
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
    key: u32,
    /// The rectangle (`[y, x, height, width]`) of the whole update.
    rect: [usize; 4],
    parts: VecDeque<Messages>,
}

struct State {
    waiting: Option<Waiting>,
    sync: SyncState,
    /// Acknowledgements the connection synchronization still waits for.
    sync_acks: usize,
}

/// Schedules the acknowledged transfers of one state on its connection.
///
/// One transfer is in flight at a time. A producer waits until the state is
/// idle, with nothing in flight and nothing waiting, and then sends. Only a
/// split update leaves work behind: its first part is sent and the others
/// wait, each sent when the client acknowledged the one before. A set of the
/// same key drops those waiting parts, because it overwrites all of them, and
/// so does a forced update whose rectangle covers theirs.
///
/// `event` is set while the state is idle. Producers are serialized by
/// `lock`, which they hold while they wait, so the changes of one state reach
/// the client in the order they were made.
pub(super) struct Scheduler {
    pub(super) id: u64,
    pub(super) lock: Mutex<()>,
    state: Mutex<State>,
    pub(super) sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl Scheduler {
    pub(super) fn new(id: u64, sender: MessageSender, connected: Arc<AtomicBool>) -> Self {
        let event = Event::new();
        event.set();
        Self {
            id,
            lock: Mutex::new(()),
            state: Mutex::new(State {
                waiting: None,
                sync: SyncState::new(),
                sync_acks: 0,
            }),
            sender,
            connected,
            event,
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

    /// Whether parts of a split update wait behind the part in flight.
    #[cfg(all(test, feature = "client"))]
    pub(super) fn has_waiting(&self) -> bool {
        self.state.lock().waiting.is_some()
    }

    /// Whether nothing is in flight and nothing waits, so a change would be
    /// sent at once. Also true while no client is synchronized, because a
    /// change is then only stored.
    pub(super) fn is_idle(&self) -> bool {
        let state = self.state.lock();
        state.sync.sendable(&self.connected).is_none() || self.event.is_set()
    }

    /// Sends a set of `key` once the state is idle. Waiting parts of the same
    /// key are dropped first: the set overwrites them.
    pub(super) fn send_set(
        &self,
        lock: MutexGuard<'_, ()>,
        key: u32,
        messages: Messages,
        wait: bool,
        generation: u64,
    ) {
        {
            let mut state = self.state.lock();
            if state.sync.generation() != generation {
                return;
            }
            if state
                .waiting
                .as_ref()
                .is_some_and(|waiting| waiting.key == key)
            {
                state.waiting = None;
            }
        }
        self.send(
            lock,
            key,
            [0; 4],
            VecDeque::from([messages]),
            wait,
            generation,
        );
    }

    /// Sends the parts of an update of `key` once the state is idle. With
    /// `force`, waiting parts of the same key are dropped first when `rect`
    /// covers their rectangle.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn send_update(
        &self,
        lock: MutexGuard<'_, ()>,
        key: u32,
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
                    .as_ref()
                    .is_some_and(|waiting| waiting.key == key && covers(rect, waiting.rect))
            {
                state.waiting = None;
            }
        }
        self.send(lock, key, rect, parts, wait, generation);
    }

    /// Waits until the state is idle, sends the first part and leaves the
    /// others waiting. With `wait`, also waits until the client acknowledged
    /// the last part.
    fn send(
        &self,
        _lock: MutexGuard<'_, ()>,
        key: u32,
        rect: [usize; 4],
        mut parts: VecDeque<Messages>,
        wait: bool,
        generation: u64,
    ) {
        self.event.wait_clear();
        // Keep reset from changing the generation between this check and the
        // enqueue, which could otherwise leak stale work into a new
        // connection. Stale work leaves the event clear: the next
        // synchronization sets it again.
        let mut state = self.state.lock();
        if state.sync.generation() != generation || !self.is_connected() {
            return;
        }
        let Some(first) = parts.pop_front() else {
            self.event.set();
            return;
        };
        if !parts.is_empty() {
            state.waiting = Some(Waiting { key, rect, parts });
        }
        self.send_all(first);
        drop(state);

        // The producer lock is still held, so only this transfer's
        // acknowledgements or a disconnect set the event.
        if wait {
            self.event.wait();
        }
    }

    /// Drops the waiting parts of `key`, or of every key for `None`, then
    /// queues `message`, which the client does not acknowledge, once the
    /// state is idle.
    pub(super) fn send_control(
        &self,
        _lock: MutexGuard<'_, ()>,
        key: Option<u32>,
        message: FastVec<32>,
        generation: u64,
    ) {
        {
            let mut state = self.state.lock();
            if key.is_none_or(|key| state.waiting.as_ref().is_some_and(|w| w.key == key)) {
                state.waiting = None;
            }
        }
        self.event.wait();
        let state = self.state.lock();
        if state.sync.generation() != generation || !self.is_connected() {
            return;
        }
        self.sender.send(message);
    }

    /// Queues `message`, which the client does not acknowledge, at once: it
    /// follows the transfers of earlier producers on the wire without waiting
    /// for their acknowledgement. Only for states that never leave split
    /// parts waiting, which it would overtake; others use
    /// [`send_control`](Self::send_control).
    pub(super) fn send_unacked(
        &self,
        _lock: MutexGuard<'_, ()>,
        message: FastVec<32>,
        generation: u64,
    ) {
        let state = self.state.lock();
        debug_assert!(state.waiting.is_none(), "would overtake waiting parts");
        if state.sync.generation() != generation || !self.is_connected() {
            return;
        }
        self.sender.send(message);
    }

    /// Starts a connection synchronization that sends `messages` and waits
    /// for `ack_count` acknowledgements.
    pub(super) fn begin_sync(&self, messages: Messages, ack_count: usize) {
        let mut state = self.state.lock();
        state.sync.synchronized();
        state.waiting = None;
        state.sync_acks = ack_count;
        if ack_count == 0 {
            self.event.set();
        } else {
            self.event.clear();
        }
        self.send_all(messages);
    }

    pub(super) fn acknowledge(&self) {
        let mut state = self.state.lock();
        if state.sync_acks > 0 {
            state.sync_acks -= 1;
            if state.sync_acks > 0 {
                return;
            }
        }

        let next = state
            .waiting
            .as_mut()
            .and_then(|waiting| waiting.parts.pop_front());
        if state
            .waiting
            .as_ref()
            .is_some_and(|waiting| waiting.parts.is_empty())
        {
            state.waiting = None;
        }
        match next {
            Some(part) => self.send_all(part),
            None => self.event.set(),
        }
    }

    pub(super) fn reset(&self) {
        // Bump the generation before releasing a waiting producer, so it sees
        // that its work belongs to the old connection.
        {
            let mut state = self.state.lock();
            state.waiting = None;
            state.sync_acks = 0;
            state.sync.reset();
        }
        self.event.set();
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
        scheduler.begin_sync(Vec::new(), 0);
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
        scheduler.send_set(scheduler.lock.lock(), key, part(value), wait, generation);
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
        scheduler.send_update(
            scheduler.lock.lock(),
            key,
            rect,
            parts,
            force,
            wait,
            generation,
        );
    }

    fn control(scheduler: &Scheduler, key: Option<u32>, value: u8) {
        let generation = scheduler.generation();
        scheduler.send_control(scheduler.lock.lock(), key, marker(value), generation);
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

        scheduler.begin_sync(part(1), 1);
        assert_marker(&mut receiver, 1);
        assert!(!scheduler.is_idle());
        scheduler.acknowledge();
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
        scheduler.acknowledge();
        finished(&done);
        assert_marker(&mut receiver, 2);
        scheduler.acknowledge();
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
            scheduler.acknowledge();
            assert_marker(&mut receiver, value);
            assert_blocked(&done);
        }
        scheduler.acknowledge();
        finished(&done);
        assert_marker(&mut receiver, 4);
        scheduler.acknowledge();
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
        scheduler.acknowledge();
        finished(&done);
        assert_marker(&mut receiver, 4);
        assert_no_message(&mut receiver);
        scheduler.acknowledge();
        assert!(scheduler.is_idle());

        update(&scheduler, 7, RECT, &[5, 6], false, false);
        assert_marker(&mut receiver, 5);
        let done = spawn_blocked(&scheduler, |s| set(s, 9, 7, false));
        scheduler.acknowledge();
        assert_marker(&mut receiver, 6);
        assert_blocked(&done);
        scheduler.acknowledge();
        finished(&done);
        assert_marker(&mut receiver, 7);
        scheduler.acknowledge();
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
        scheduler.acknowledge();
        finished(&done);
        for value in [4, 5] {
            assert_marker(&mut receiver, value);
            assert_no_message(&mut receiver);
            scheduler.acknowledge();
        }
        assert!(scheduler.is_idle());

        // A rectangle that does not cover, or another key: every part is sent.
        for (key, rect) in [(7, [0, 0, 1, 1]), (9, RECT)] {
            update(&scheduler, 7, [0, 0, 2, 2], &[6, 7], false, false);
            assert_marker(&mut receiver, 6);
            let done = spawn_blocked(&scheduler, move |s| update(s, key, rect, &[8], true, false));
            scheduler.acknowledge();
            assert_marker(&mut receiver, 7);
            assert_blocked(&done);
            scheduler.acknowledge();
            finished(&done);
            assert_marker(&mut receiver, 8);
            scheduler.acknowledge();
            assert!(scheduler.is_idle());
        }
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
        scheduler.acknowledge();
        finished(&done);
        assert_marker(&mut receiver, 10);
        assert_no_message(&mut receiver);
        assert!(scheduler.is_idle());

        // Removing another key waits for every part.
        update(&scheduler, 7, RECT, &[2, 3], false, false);
        assert_marker(&mut receiver, 2);
        let done = spawn_blocked(&scheduler, |s| control(s, Some(9), 11));
        scheduler.acknowledge();
        assert_marker(&mut receiver, 3);
        assert_blocked(&done);
        scheduler.acknowledge();
        finished(&done);
        assert_marker(&mut receiver, 11);

        // A control of every key drops every part.
        update(&scheduler, 7, RECT, &[4, 5], false, false);
        assert_marker(&mut receiver, 4);
        let done = spawn_blocked(&scheduler, |s| control(s, None, 12));
        scheduler.acknowledge();
        finished(&done);
        assert_marker(&mut receiver, 12);
        assert_no_message(&mut receiver);

        // Unacknowledged controls leave the state idle.
        assert!(scheduler.is_idle());
        set(&scheduler, 9, 13, false);
        assert_marker(&mut receiver, 13);
    }

    #[test]
    fn an_unacknowledged_message_is_queued_behind_the_transfer_in_flight() {
        let (scheduler, _, mut receiver) = scheduler();
        set(&scheduler, 7, 1, false);
        assert_marker(&mut receiver, 1);

        let generation = scheduler.generation();
        scheduler.send_unacked(scheduler.lock.lock(), marker(2), generation);
        assert_marker(&mut receiver, 2);
        assert!(!scheduler.is_idle(), "the set is still in flight");
        scheduler.acknowledge();
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
        scheduler.acknowledge();
        finished(&done);
        assert!(scheduler.is_idle());

        let done = spawn_blocked(&scheduler, |s| update(s, 7, RECT, &[2, 3], false, true));
        assert_marker(&mut receiver, 2);
        scheduler.acknowledge();
        assert_marker(&mut receiver, 3);
        assert_blocked(&done);
        scheduler.acknowledge();
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
        scheduler.begin_sync(Vec::new(), 0);
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
        scheduler.begin_sync(Vec::new(), 0);

        scheduler.send_set(scheduler.lock.lock(), 7, part(1), false, stale);
        let parts = VecDeque::from([part(2), part(3)]);
        scheduler.send_update(scheduler.lock.lock(), 7, RECT, parts, false, false, stale);
        scheduler.send_control(scheduler.lock.lock(), None, marker(4), stale);
        scheduler.send_unacked(scheduler.lock.lock(), marker(6), stale);
        assert_no_message(&mut receiver);
        assert!(scheduler.is_idle(), "stale work consumed the idle state");

        set(&scheduler, 7, 5, false);
        assert_marker(&mut receiver, 5);
    }
}
