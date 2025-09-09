use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

pub struct Event {
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

    pub fn set(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
    }

    pub fn clear(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    pub async fn wait(&self) {
        loop {
            if self.flag.load(Ordering::Relaxed) {
                return;
            }
            self.notify.notified().await;
        }
    }

    pub async fn wait_lock(&self) {
        loop {
            if self.flag.load(Ordering::Relaxed) {
                self.flag.store(false, Ordering::Relaxed);
                return;
            }
            self.notify.notified().await;
        }
    }
}
