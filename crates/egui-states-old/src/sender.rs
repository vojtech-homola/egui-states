use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use egui_states_core_old::serialization::MessageData;

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<Option<MessageData>>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, UnboundedReceiver<Option<MessageData>>) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    pub(crate) fn send(&self, msg: MessageData) {
        self.sender.send(Some(msg)).unwrap();
    }

    pub(crate) fn close(&self) {
        self.sender.send(None).unwrap();
    }
}
