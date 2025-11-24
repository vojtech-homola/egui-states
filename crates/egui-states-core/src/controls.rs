use serde::{Deserialize, Serialize};

use crate::nohash::NoHashMap;

#[derive(Serialize, Deserialize)]
pub enum ControlServer {
    Error(String),
    TypesAsk(NoHashMap<u64, u64>),
    Update(f32),
}

#[derive(Serialize, Deserialize)]
pub enum ControlClient {
    Error(String),
    Ack(u64),
    TypesAnswer(Vec<u64>),
    Handshake(u64, u64),
}

impl ControlClient {
    pub fn as_str(&self) -> &str {
        match self {
            ControlClient::Error(_) => "ErrorCommand",
            ControlClient::Ack(_) => "AckCommand",
            ControlClient::TypesAnswer(_) => "TypesAnswerCommand",
            ControlClient::Handshake(_, _) => "HandshakeCommand",
        }
    }
}
