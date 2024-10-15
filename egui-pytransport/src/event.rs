use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub struct Event {
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
    pub fn new() -> Self {
        Self {
            cond: Arc::new(Condvar::new()),
            flag: Arc::new(Mutex::new(false)),
        }
    }

    pub fn set(&self) {
        *self.flag.lock().unwrap() = true;
        self.cond.notify_all();
    }

    pub fn set_one(&self) {
        *self.flag.lock().unwrap() = true;
        self.cond.notify_one();
    }

    pub fn is_set(&self) -> bool {
        *self.flag.lock().unwrap()
    }

    pub fn clear(&self) {
        *self.flag.lock().unwrap() = false;
    }

    pub fn wait(&self) -> bool {
        *self
            .cond
            .wait_while(self.flag.lock().unwrap(), |flag| !*flag)
            .unwrap()
    }

    pub fn wait_lock(&self) -> bool {
        *self
            .cond
            .wait_while(self.flag.lock().unwrap(), |flag| {
                if *flag {
                    *flag = false;
                    false
                } else {
                    true
                }
            })
            .unwrap()
    }

    pub fn wait_timeout(&self, time_out: f32) -> bool {
        let duration = Duration::from_secs_f32(time_out);

        let (result, _time_out_result) = self
            .cond
            .wait_timeout_while(self.flag.lock().unwrap(), duration, |flag| !*flag)
            .unwrap();

        *result
    }

    pub fn wait_timeout_lock(&self, time_out: f32) -> bool {
        let duration = Duration::from_secs_f32(time_out);

        let (result, _time_out_result) = self
            .cond
            .wait_timeout_while(self.flag.lock().unwrap(), duration, |flag| {
                if *flag {
                    *flag = false;
                    false
                } else {
                    true
                }
            })
            .unwrap();

        *result
    }
}
