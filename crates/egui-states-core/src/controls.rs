use serde::{Deserialize, Serialize};

use crate::nohash::NoHashMap;

#[derive(Serialize, Deserialize)]
pub enum ControlServer {
    Error,
    Update(f32),
}

#[derive(Serialize, Deserialize)]
pub enum ControlClient {
    Error(String),
    Ack(u64),
    Handshake(u16, u64, NoHashMap<u64, u64>),
}

impl ControlClient {
    pub fn as_str(&self) -> &str {
        match self {
            ControlClient::Error(_) => "ErrorCommand",
            ControlClient::Ack(_) => "AckCommand",
            ControlClient::Handshake(_, _, _) => "HandshakeCommand",
        }
    }
}
