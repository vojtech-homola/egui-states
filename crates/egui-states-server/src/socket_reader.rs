use bytes::Bytes;
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use egui_states_core::nohash::NoHashMap;

pub(crate) enum ClientMessage {
    Value(u64, bool, Bytes),
    Signal(u64, Bytes),
    Ack(u64),
    Error(String),
    Handshake(u16, u64, NoHashMap<u64, u64>)
}

pub(crate) struct SocketReader {
    socket: SplitStream<WebSocketStream<TcpStream>>,
    previous: Option<(Bytes, usize)>,
}

impl SocketReader {
    pub(crate) fn new(socket: SplitStream<WebSocketStream<TcpStream>>) -> Self {
        Self {
            socket,
            previous: None,
        }
    }

    pub(crate) async fn next(&mut self) -> Result<ClientMessage, String> {
        if let Some((data, pointer)) = self.previous {

        }
    }
}
