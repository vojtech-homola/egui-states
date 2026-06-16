use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use event_listener::Event as ListenerEvent;
#[cfg(feature = "server")]
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

    #[cfg(feature = "server")]
    pub(crate) fn is_set(&self) -> bool {
        self.0.flag.load(Ordering::Acquire)
    }

    #[cfg(feature = "server")]
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

    #[cfg(feature = "server")]
    pub(crate) fn wait(&self) {
        loop {
            if self.0.flag.load(Ordering::Acquire) {
                return;
            }

            let listener = self.0.notify.listen();

            if self.0.flag.load(Ordering::Acquire) {
                return;
            }

            listener.wait();
        }
    }

    #[cfg(feature = "server")]
    pub(crate) fn wait_clear(&self) {
        loop {
            if self
                .0
                .flag
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }

            let listener = self.0.notify.listen();

            if self
                .0
                .flag
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }

            listener.wait();
        }
    }

    pub(crate) async fn wait_clear_async(&self) {
        loop {
            if self
                .0
                .flag
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }

            let listener = self.0.notify.listen();

            if self
                .0
                .flag
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }

            listener.await;
        }
    }
}
