use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use egui_states_core::serialization::{ClientHeader, MessageData};

enum CahnnelMessage {
    Value(u64, bool, MessageData),
    Signal(u64, MessageData),
    Ack(u64),
    Error(String),
    Close,
}

pub(crate) type ChannelMessage = Option<(ClientHeader, Option<MessageData>)>;

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<ChannelMessage>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, UnboundedReceiver<ChannelMessage>) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    pub(crate) fn send_data(&self, header: ClientHeader, data: MessageData) {
        self.sender.send(Some((header, Some(data)))).unwrap();
    }

    pub(crate) fn send(&self, header: ClientHeader) {
        self.sender.send(Some((header, None))).unwrap();
    }

    pub(crate) fn close(&self) {
        self.sender.send(None).unwrap();
    }
}
