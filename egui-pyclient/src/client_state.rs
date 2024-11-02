use std::time::Duration;

use egui::Context;

use egui_pysync::event::Event;

#[derive(Clone)]
pub struct UIState {
    context: Context,
    connect_signal: Event,
}

impl UIState {
    pub fn new(context: Context) -> Self {
        Self {
            context,
            connect_signal: Event::new(),
        }
    }

    pub fn update(&self, time: f32) {
        if time > 0.0 {
            self.context
                .request_repaint_after(Duration::from_secs_f32(time));
        } else {
            self.context.request_repaint();
        }
    }

    pub fn connect_signal(&self) -> &Event {
        &self.connect_signal
    }
}
