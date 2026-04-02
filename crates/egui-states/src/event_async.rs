use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

pub(crate) struct Event {
    notify: Arc<Notify>,
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
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(feature = "server")]
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    pub fn set(&self) {
        self.flag.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    pub fn clear(&self) {
        self.flag.store(false, Ordering::Release);
    }

    pub async fn wait_clear(&self) {
        let notified = self.notify.notified();
        tokio::pin!(notified);

        loop {
            notified.as_mut().enable();
            if self.flag.fetch_and(false, Ordering::AcqRel) {
                return;
            }
            notified.as_mut().await;
            notified.set(self.notify.notified());
        }
    }
}
