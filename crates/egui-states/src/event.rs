use std::sync::{Arc, RwLock};

use tokio::sync::Notify;

pub(crate) struct Event {
    notify: Arc<Notify>,
    flag: Arc<RwLock<bool>>,
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
            flag: Arc::new(RwLock::new(false)),
        }
    }

    pub fn set(&self) {
        *self.flag.write().unwrap() = true;
        self.notify.notify_waiters();
    }

    pub fn clear(&self) {
        *self.flag.write().unwrap() = false;
    }

    pub async fn wait_lock(&self) {
        loop {
            if *self.flag.read().unwrap() {
                *self.flag.write().unwrap() = false;
                return;
            }
            self.notify.notified().await;
        }
    }
}
