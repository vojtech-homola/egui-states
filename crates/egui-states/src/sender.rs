use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use egui_states_core::serialization::MessageData;

pub(crate) enum ChannelMessage {
    Value(u64, bool, MessageData),
    Signal(u64, MessageData),
    Ack(u64),
    Error(String),
}

// pub(crate) type ChannelMessage = Option<(ClientHeader, Option<MessageData>)>;

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<Option<ChannelMessage>>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, UnboundedReceiver<Option<ChannelMessage>>) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    // pub(crate) fn send_data(&self, header: ClientHeader, data: MessageData) {
    //     self.sender.send(Some((header, Some(data)))).unwrap();
    // }

    pub(crate) fn send(&self, msg: ChannelMessage) {
        self.sender.send(Some(msg)).unwrap();
    }

    pub(crate) fn close(&self) {
        self.sender.send(None).unwrap();
    }
}
