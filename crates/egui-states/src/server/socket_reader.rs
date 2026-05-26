use bytes::Bytes;
use futures_util::{StreamExt, stream::SplitStream};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::serialization::ClientHeader;

const COPY_SIZE: usize = 1024; // 1 KB

pub(crate) enum ClientMessage {
    Value(u64, u32, bool, Bytes),
    Signal(u64, u32, Bytes),
    Ack(u64),
    Handshake(u16, u64),
}

pub(crate) struct SocketReader {
    socket: SplitStream<WebSocketStream<TcpStream>>,
    previous: Option<(Bytes, usize, bool)>,
}

impl SocketReader {
    pub(crate) fn new(socket: SplitStream<WebSocketStream<TcpStream>>) -> Self {
        Self {
            socket,
            previous: None,
        }
    }

    pub(crate) async fn next(&mut self) -> Result<ClientMessage, String> {
        let (data, pointer, copy) = match self.previous.take() {
            Some(prev) => prev,
            None => match self.socket.next().await {
                Some(Ok(Message::Binary(msg))) => {
                    let copy = msg.len() > COPY_SIZE;
                    (msg, 0, copy)
                }
                Some(Ok(_)) => return Err("Received non-binary message".to_string()),
                Some(Err(e)) => return Err(format!("Reading message from client failed: {:?}", e)),
                None => return Err("Connection was closed by the client".to_string()),
            },
        };

        let (header, size) = ClientHeader::deserialize(&data[pointer..])
            .map_err(|_| "Failed to deserialize message header".to_string())?;
        match header {
            ClientHeader::Value(id, type_id, signal, data_size) => {
                let all_size = size + data_size as usize;
                if all_size > data.len() - pointer {
                    return Err("Incomplete data received".to_string());
                }
                let header_data = if copy {
                    data.slice(pointer + size..pointer + all_size)
                } else {
                    Bytes::copy_from_slice(&data[pointer + size..pointer + all_size])
                };
                if pointer + all_size < data.len() {
                    self.previous = Some((data, pointer + all_size, copy));
                }
                Ok(ClientMessage::Value(id, type_id, signal, header_data))
            }
            ClientHeader::Signal(id, type_id, data_size) => {
                let all_size = size + data_size as usize;
                if all_size > data.len() - pointer {
                    return Err("Incomplete data received".to_string());
                }
                let header_data = if copy {
                    data.slice(pointer + size..pointer + all_size)
                } else {
                    Bytes::copy_from_slice(&data[pointer + size..pointer + all_size])
                };
                if pointer + all_size < data.len() {
                    self.previous = Some((data, pointer + all_size, copy));
                }
                Ok(ClientMessage::Signal(id, type_id, header_data))
            }
            ClientHeader::Ack(id) => {
                if pointer + size < data.len() {
                    self.previous = Some((data, pointer + size, copy));
                }
                Ok(ClientMessage::Ack(id))
            }
            ClientHeader::Handshake(protocol_version, client_id) => {
                if pointer + size < data.len() {
                    self.previous = Some((data, pointer + size, copy));
                }
                Ok(ClientMessage::Handshake(protocol_version, client_id))
            }
        }
    }
}
