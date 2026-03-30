use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use parking_lot::Condvar;
use parking_lot::Mutex;
use tokio::sync::Notify;

pub(crate) struct EventUniversal {
    notify: Arc<Notify>,
    flag: Arc<Mutex<bool>>,
    #[cfg(not(target_arch = "wasm32"))]
    cond: Arc<Condvar>,
}

impl Clone for EventUniversal {
    fn clone(&self) -> Self {
        Self {
            notify: self.notify.clone(),
            flag: self.flag.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            cond: self.cond.clone(),
        }
    }
}

impl EventUniversal {
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            flag: Arc::new(Mutex::new(false)),
            #[cfg(not(target_arch = "wasm32"))]
            cond: Arc::new(Condvar::new()),
        }
    }

    pub fn is_set(&self) -> bool {
        *self.flag.lock()
    }

    pub fn set(&self) {
        *self.flag.lock() = true;
        self.notify.notify_waiters();
        #[cfg(not(target_arch = "wasm32"))]
        self.cond.notify_all();
    }

    pub fn clear(&self) {
        *self.flag.lock() = false;
    }

    pub async fn wait_clear(&self) {
        let notified = self.notify.notified();
        tokio::pin!(notified);

        loop {
            notified.as_mut().enable();
            if self.try_clear() {
                return;
            }
            notified.as_mut().await;
            notified.set(self.notify.notified());
        }
    }

    fn try_clear(&self) -> bool {
        let mut flag = self.flag.lock();
        if *flag {
            *flag = false;
            true
        } else {
            false
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_clear_blocking(&self) {
        self.cond.wait_while(&mut self.flag.lock(), |flag| {
            if *flag {
                *flag = false;
                false
            } else {
                true
            }
        });
    }
}
