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

struct ClientInner {
    context: Option<Context>,
    connect_signal: Event,
    state: Arc<RwLock<ConnectionState>>,
    sender: MessageSender,
}

impl ClientInner {
    fn set_context(&mut self, context: Context) {
        self.context.replace(context);
    }
}

#[derive(Clone)]
pub struct Client(Arc<ClientInner>);

impl Client {
    pub(crate) fn new(context: Option<Context>, sender: MessageSender) -> Self {
        let inner = ClientInner {
            context,
            connect_signal: Event::new(),
            state: Arc::new(RwLock::new(ConnectionState::NotConnected)),
            sender,
        };

        Self(Arc::new(inner))
    }

    pub fn set_context(&mut self, context: Context) {
        Arc::get_mut(&mut self.0).unwrap().set_context(context);
    }

    pub fn update(&self, time: f32) {
        if let Some(ctx) = &self.0.context {
            if time > 0.0 {
                ctx.request_repaint_after(Duration::from_secs_f32(time));
            } else {
                ctx.request_repaint();
            }
        }
    }

    pub(crate) async fn wait_connection(&self) {
        self.0.connect_signal.clear();
        self.0.connect_signal.wait_clear().await;
    }

    pub fn connect(&self) {
        self.0.connect_signal.set();
    }

    pub fn disconnect(&self) {
        self.0.sender.close();
    }

    pub(crate) fn set_state(&self, state: ConnectionState) {
        *self.0.state.write() = state;
        if let Some(ctx) = &self.0.context {
            ctx.request_repaint();
        }
    }

    pub fn get_state(&self) -> ConnectionState {
        *self.0.state.read()
    }
}
