use parking_lot::{Condvar, Mutex};
use std::sync::Arc;

pub(crate) struct Event {
    cond: Arc<Condvar>,
    flag: Arc<Mutex<bool>>,
}

impl Clone for Event {
    fn clone(&self) -> Self {
        Self {
            cond: self.cond.clone(),
            flag: self.flag.clone(),
        }
    }
}

impl Event {
    pub(crate) fn new() -> Self {
        Self {
            cond: Arc::new(Condvar::new()),
            flag: Arc::new(Mutex::new(false)),
        }
    }

    pub(crate) fn set_one(&self) {
        *self.flag.lock() = true;
        self.cond.notify_one();
    }

    pub(crate) fn wait_lock(&self) {
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
