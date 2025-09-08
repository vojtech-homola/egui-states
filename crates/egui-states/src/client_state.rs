use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use egui::Context;

use crate::event::Event;
use crate::sender::MessageSender;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    NotConnected,
    Connected,
    Disconnected,
}

#[derive(Clone)]
pub struct UIState {
    context: Context,
    connect_signal: Event,
    state: Arc<RwLock<ConnectionState>>,
    sender: MessageSender,
}

impl UIState {
    pub(crate) fn new(context: Context, sender: MessageSender) -> Self {
        Self {
            context,
            connect_signal: Event::new(),
            state: Arc::new(RwLock::new(ConnectionState::NotConnected)),
            sender,
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

    pub(crate) async fn wait_connection(&self) {
        self.connect_signal.clear();
        self.connect_signal.wait_lock().await;
    }

    pub fn connect(&self) {
        self.connect_signal.set();
    }

    pub fn disconnect(&self) {
        self.sender.close();
    }

    pub(crate) fn set_state(&self, state: ConnectionState) {
        *self.state.write().unwrap() = state;
        self.context.request_repaint();
    }

    pub fn get_state(&self) -> ConnectionState {
        *self.state.read().unwrap()
    }
}
