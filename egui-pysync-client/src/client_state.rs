use std::sync::{Arc, RwLock};
use std::time::Duration;

use egui::Context;

use egui_pysync_common::event::Event;

#[derive(Clone)]
pub struct UIState {
    context: Arc<RwLock<Option<Context>>>,
    connect_signal: Event,
}

impl UIState {
    pub fn new() -> Self {
        Self {
            context: Arc::new(RwLock::new(None)),
            connect_signal: Event::new(),
        }
    }

    pub fn set_context(&mut self, context: Context) {
        self.context.write().unwrap().replace(context);
    }

    pub fn update(&self, time: f32) {
        if let Some(context) = self.context.read().unwrap().as_ref() {
            if time > 0.0 {
                context.request_repaint_after(Duration::from_secs_f32(time));
            } else {
                context.request_repaint();
            }
        }
    }

    pub fn connect_signal(&self) -> &Event {
        &self.connect_signal
    }
}
