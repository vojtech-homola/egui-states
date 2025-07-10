use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) enum CommandMessage {
    Error(String),
    Ack(u32),
    Handshake(u64, u64),
    Update(f32),
}

#[cfg(feature = "server")]
impl CommandMessage {
    pub fn as_str(&self) -> &str {
        match self {
            CommandMessage::Error(_) => "ErrorCommand",
            CommandMessage::Ack(_) => "AckCommand",
            CommandMessage::Handshake(_, _) => "HandshakeCommand",
            CommandMessage::Update(_) => "UpdateCommand",
        }
    }
}
