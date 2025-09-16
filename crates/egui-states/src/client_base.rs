use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

use egui::Context;

use egui_states_core::event_async::Event;

use crate::sender::MessageSender;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    NotConnected,
    Connected,
    Disconnected,
}

#[derive(Clone)]
pub struct Client {
    context: Option<Context>,
    connect_signal: Event,
    state: Arc<RwLock<ConnectionState>>,
    sender: MessageSender,
}

impl Client {
    pub(crate) fn new(context: Option<Context>, sender: MessageSender) -> Self {
        Self {
            context,
            connect_signal: Event::new(),
            state: Arc::new(RwLock::new(ConnectionState::NotConnected)),
            sender,
        }
    }

    pub fn set_context(&mut self, context: Context) {
        self.context = Some(context);
    }

    pub fn update(&self, time: f32) {
        if let Some(ctx) = &self.context {
            if time > 0.0 {
                ctx.request_repaint_after(Duration::from_secs_f32(time));
            } else {
                ctx.request_repaint();
            }
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
        *self.state.write() = state;
        if let Some(ctx) = &self.context {
            ctx.request_repaint();
        }
    }

    pub fn get_state(&self) -> ConnectionState {
        *self.state.read()
    }
}
