use crate::sender::MessageSender;

pub struct ValueDataServer {
    id: u32,
    sender: MessageSender,
}

impl ValueDataServer {
    pub(crate) async fn update(&self, data: &[u8]) -> Result<(), String> {
        Ok(())
    }
}
