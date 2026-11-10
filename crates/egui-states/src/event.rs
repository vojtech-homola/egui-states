use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[cfg(any(feature = "server", feature = "python"))]
use std::time::Instant;

use event_listener::Event as ListenerEvent;
#[cfg(any(feature = "server", feature = "python"))]
use event_listener::Listener;

struct Inner {
    notify: ListenerEvent,
    flag: AtomicBool,
}

pub(crate) struct Event(Arc<Inner>);

impl Clone for Event {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Event {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Inner {
            notify: ListenerEvent::new(),
            flag: AtomicBool::new(false),
        }))
    }

    #[cfg(any(feature = "server", feature = "python", feature = "client"))]
    pub(crate) fn is_set(&self) -> bool {
        self.0.flag.load(Ordering::Acquire)
    }

    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn set_one(&self) {
        self.0.flag.store(true, Ordering::Release);
        self.0.notify.notify(1);
    }

    pub fn set(&self) {
        self.0.flag.store(true, Ordering::Release);
        self.0.notify.notify(usize::MAX);
    }

    pub fn clear(&self) {
        self.0.flag.store(false, Ordering::Release);
    }

    /// Checks the flag; with `consume`, also clears it if it was set.
    #[inline]
    fn take(&self, consume: bool) -> bool {
        if consume {
            self.0
                .flag
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        } else {
            self.0.flag.load(Ordering::Acquire)
        }
    }

    /// The blocking wait every variant below uses.
    ///
    /// Returns once the flag is set (clearing it with `consume`), once `stop`
    /// is raised, or at `deadline`. The flag and `stop` are checked again
    /// after the listener is registered, so a `set` that lands between the
    /// first check and the registration is not missed.
    #[cfg(any(feature = "server", feature = "python"))]
    fn wait_for(
        &self,
        consume: bool,
        stop: Option<&AtomicBool>,
        deadline: Option<Instant>,
    ) -> WaitOutcome {
        let stopped = || stop.is_some_and(|stop| stop.load(Ordering::Acquire));
        loop {
            if stopped() {
                return WaitOutcome::Stopped;
            }
            if self.take(consume) {
                return WaitOutcome::Set;
            }

            let listener = self.0.notify.listen();

            if stopped() {
                return WaitOutcome::Stopped;
            }
            if self.take(consume) {
                return WaitOutcome::Set;
            }

            match deadline {
                Some(deadline) => {
                    if listener.wait_deadline(deadline).is_none() {
                        return WaitOutcome::TimedOut;
                    }
                }
                None => listener.wait(),
            }
        }
    }

    /// The async twin of [`Self::wait_for`], without stop flag or deadline.
    async fn wait_for_async(&self, consume: bool) {
        loop {
            if self.take(consume) {
                return;
            }

            let listener = self.0.notify.listen();

            if self.take(consume) {
                return;
            }

            listener.await;
        }
    }

    /// Waits until the event is set, leaving it set.
    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn wait(&self) {
        self.wait_for(false, None, None);
    }

    /// Waits until the event is set and clears it.
    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn wait_clear(&self) {
        self.wait_for(true, None, None);
    }

    /// Like [`Self::wait_clear`], but returns early once `stop` is raised or
    /// at `deadline`.
    ///
    /// `None` waits without a deadline.
    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn wait_clear_deadline(
        &self,
        stop: &AtomicBool,
        deadline: Option<Instant>,
    ) -> WaitOutcome {
        self.wait_for(true, Some(stop), deadline)
    }

    pub(crate) async fn wait_clear_async(&self) {
        self.wait_for_async(true).await
    }

    /// Waits until the event is set, leaving it set.
    #[cfg(feature = "client")]
    pub(crate) async fn wait_async(&self) {
        self.wait_for_async(false).await
    }
}

/// Result of a blocking wait.
#[cfg(any(feature = "server", feature = "python"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitOutcome {
    /// The event was set and has been cleared by this waiter.
    Set,
    /// The stop flag was raised.
    Stopped,
    /// The deadline passed before the event was set.
    TimedOut,
}
