use serde::{Deserialize, Serialize};

use crate::nohash::NoHashMap;

#[derive(Serialize, Deserialize)]
pub enum ControlMessage {
    Error(String),
    Ack(u64),
    TypesAsk(NoHashMap<u64, u64>),
    TypesAnswer(NoHashMap<u64, bool>),
    Handshake(u64, u64),
    Update(f32),
}

impl ControlMessage {
    pub fn as_str(&self) -> &str {
        match self {
            ControlMessage::Error(_) => "ErrorCommand",
            ControlMessage::Ack(_) => "AckCommand",
            ControlMessage::TypesAsk(_) => "TypesAskCommand",
            ControlMessage::TypesAnswer(_) => "TypesAnswerCommand",
            ControlMessage::Handshake(_, _) => "HandshakeCommand",
            ControlMessage::Update(_) => "UpdateCommand",
        }
    }
}
