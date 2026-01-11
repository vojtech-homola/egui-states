use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use egui_states_core::serialization::FastVec;

pub(crate) type SenderData = FastVec<32>;
pub(crate) type MessageReceiver = UnboundedReceiver<Option<SenderData>>;

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<Option<SenderData>>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, MessageReceiver) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    pub(crate) fn send(&self, msg: SenderData) {
        let _ = self.sender.send(Some(msg));
    }

    pub(crate) fn close(&self) {
        let _ = self.sender.send(None);
    }
}
