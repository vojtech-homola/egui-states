use egui_states_core::serialization::MessageData;

pub trait ChannelMessage: Send + Sync + 'static {
    fn send(&self, message: MessageData);
    fn close(&self);
}
