use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Bytes;

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<Option<Bytes>>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, UnboundedReceiver<Option<Bytes>>) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    pub(crate) fn send(&self, msg: Bytes) {
        self.sender.send(Some(msg)).unwrap();
    }

    pub(crate) fn close(&self) {
        self.sender.send(None).unwrap();
    }
}
