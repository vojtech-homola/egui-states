use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::{Bytes, Message};

pub(crate) type MessageReceiver = UnboundedReceiver<Option<Message>>;

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<Option<Message>>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, MessageReceiver) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    pub(crate) fn send(&self, msg: Bytes) {
        self.sender.send(Some(Message::from(msg))).unwrap();
    }

    pub(crate) fn close(&self) {
        self.sender.send(None).unwrap();
    }
}
