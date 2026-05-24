use event_listener::{Event as ListenerEvent, Listener};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct Event {
    notify: Arc<ListenerEvent>,
    flag: Arc<AtomicBool>,
}

impl Clone for Event {
    fn clone(&self) -> Self {
        Self {
            notify: self.notify.clone(),
            flag: self.flag.clone(),
        }
    }
}

impl Event {
    pub(crate) fn new() -> Self {
        Self {
            notify: Arc::new(ListenerEvent::new()),
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    pub(crate) fn set_one(&self) {
        self.flag.store(true, Ordering::Release);
        self.notify.notify(1);
    }

    pub(crate) fn set(&self) {
        self.flag.store(true, Ordering::Release);
        self.notify.notify(usize::MAX);
    }

    pub(crate) fn clear(&self) {
        self.flag.store(false, Ordering::Release);
    }

    pub(crate) fn wait(&self) {
        loop {
            if self.flag.load(Ordering::Acquire) {
                return;
            }

            let listener = self.notify.listen();

            if self.flag.load(Ordering::Acquire) {
                return;
            }

            listener.wait();
        }
    }

    pub(crate) fn wait_clear(&self) {
        loop {
            if self
                .flag
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }

            let listener = self.notify.listen();

            if self
                .flag
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }

            listener.wait();
        }
    }
}
