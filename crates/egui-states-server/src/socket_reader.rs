use bytes::Bytes;
use futures_util::{StreamExt, stream::SplitStream};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use egui_states_core::nohash::NoHashMap;
use egui_states_core::serialization::ClientHeader;

const COPY_SIZE: usize = 1024; // 1 KB

pub(crate) enum ClientMessage {
    Value(u64, bool, Bytes),
    Signal(u64, Bytes),
    Ack(u64),
    Error(String),
    Handshake(u16, u64, NoHashMap<u64, u64>),
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
        match self.previous.take() {
            Some((data, pointer, copy)) => {
                let (header, size) = ClientHeader::deserialize(&data[pointer..])
                    .map_err(|_| "Failed to deserialize message header".to_string())?;
                match header {
                    ClientHeader::Value(id, signal, data_size) => {
                        let all_size = size + data_size as usize;
                        if all_size > data.len() - pointer {
                            return Err("Incomplete data received".to_string());
                        }
                        let header_data = match copy {
                            true => data.slice(pointer + size..pointer + all_size),
                            false => {
                                Bytes::copy_from_slice(&data[pointer + size..pointer + all_size])
                            }
                        };
                        if pointer + all_size < data.len() {
                            self.previous = Some((data, pointer + all_size, copy));
                        }
                        Ok(ClientMessage::Value(id, signal, header_data))
                    }
                    ClientHeader::Signal(id, data_size) => {
                        let all_size = size + data_size as usize;
                        if all_size > data.len() - pointer {
                            return Err("Incomplete data received".to_string());
                        }
                        let header_data = match copy {
                            true => data.slice(pointer + size..pointer + all_size),
                            false => {
                                Bytes::copy_from_slice(&data[pointer + size..pointer + all_size])
                            }
                        };
                        if pointer + all_size < data.len() {
                            self.previous = Some((data, pointer + all_size, copy));
                        }
                        Ok(ClientMessage::Signal(id, header_data))
                    }
                    ClientHeader::Ack(id) => {
                        if pointer + size < data.len() {
                            self.previous = Some((data, pointer + size, copy));
                        }
                        Ok(ClientMessage::Ack(id))
                    }
                    ClientHeader::Error(err) => {
                        if pointer + size < data.len() {
                            self.previous = Some((data, pointer + size, copy));
                        }
                        Ok(ClientMessage::Error(err))
                    }
                    ClientHeader::Handshake(protocol_version, client_id, state_ids) => {
                        if pointer + size < data.len() {
                            self.previous = Some((data, pointer + size, copy));
                        }
                        Ok(ClientMessage::Handshake(
                            protocol_version,
                            client_id,
                            state_ids,
                        ))
                    }
                }
            }
            None => match self.socket.next().await {
                Some(Ok(Message::Binary(msg))) => {
                    let copy = msg.len() > COPY_SIZE;
                    let (header, size) = ClientHeader::deserialize(&msg)
                        .map_err(|_| "Failed to deserialize message header".to_string())?;
                    match header {
                        ClientHeader::Value(id, signal, data_size) => {
                            let all_size = size + data_size as usize;
                            if all_size > msg.len() {
                                return Err("Incomplete data received".to_string());
                            }
                            let data = match copy {
                                true => msg.slice(size..all_size),
                                false => Bytes::copy_from_slice(&msg[size..all_size]),
                            };
                            if msg.len() > all_size {
                                self.previous = Some((msg, all_size, copy));
                            }
                            Ok(ClientMessage::Value(id, signal, data))
                        }
                        ClientHeader::Signal(id, data_size) => {
                            let all_size = size + data_size as usize;
                            if all_size > msg.len() {
                                return Err("Incomplete data received".to_string());
                            }
                            let data = match copy {
                                true => msg.slice(size..all_size),
                                false => Bytes::copy_from_slice(&msg[size..all_size]),
                            };
                            if msg.len() > all_size {
                                self.previous = Some((msg, all_size, copy));
                            }
                            Ok(ClientMessage::Signal(id, data))
                        }
                        ClientHeader::Ack(id) => {
                            if size < msg.len() {
                                self.previous = Some((msg, size, copy));
                            }
                            Ok(ClientMessage::Ack(id))
                        }
                        ClientHeader::Error(err) => {
                            if size < msg.len() {
                                self.previous = Some((msg, size, copy));
                            }
                            Ok(ClientMessage::Error(err))
                        }
                        ClientHeader::Handshake(protocol_version, client_id, state_ids) => {
                            if size < msg.len() {
                                self.previous = Some((msg, size, copy));
                            }
                            Ok(ClientMessage::Handshake(
                                protocol_version,
                                client_id,
                                state_ids,
                            ))
                        }
                    }
                }
                Some(Ok(_)) => Err("Received non-binary message".to_string()),
                Some(Err(e)) => Err(format!("Reading message from client failed: {:?}", e)),
                None => Err("Connection was closed by the client".to_string()),
            },
        }
    }
}
