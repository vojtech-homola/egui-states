use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum ControlServer {
    Error,
    TypesAsk,
    Update(f32),
}

#[derive(Serialize, Deserialize)]
pub enum ControlClient {
    Error,
    Ack(u64),
    TypesAnswer,
    Handshake(u16, u64),
}

impl ControlClient {
    pub fn as_str(&self) -> &str {
        match self {
            ControlClient::Error => "ErrorCommand",
            ControlClient::Ack(_) => "AckCommand",
            ControlClient::TypesAnswer => "TypesAnswerCommand",
            ControlClient::Handshake(_, _) => "HandshakeCommand",
        }
    }
}
