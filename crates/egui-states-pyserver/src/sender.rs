use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::{Bytes, Message};

use crate::event::Event;

pub(crate) type MessageReceiver = UnboundedReceiver<Option<ServerMessage>>;

pub(crate) struct ServerMessage(pub Message, pub Option<Event>);

impl ServerMessage {
    pub fn get(self) -> (Message, Option<Event>) {
        (self.0, self.1)
    }
}

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<Option<ServerMessage>>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, MessageReceiver) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    pub(crate) fn send(&self, msg: Bytes) {
        let message = ServerMessage(Message::from(msg), None);
        self.sender.send(Some(message)).unwrap();
    }

    pub(crate) fn send_with_event(&self, msg: Bytes, event: Event) {
        let message = ServerMessage(Message::from(msg), Some(event));
        self.sender.send(Some(message)).unwrap();
    }

    pub(crate) fn close(&self) {
        self.sender.send(None).unwrap();
    }
}
