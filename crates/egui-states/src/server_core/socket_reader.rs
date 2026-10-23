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
    Message(Bytes),
    Handshake(u16, Option<u64>, Option<String>),
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

    pub(crate) async fn next(&mut self) -> Result<ClientMessage, Option<String>> {
        let (data, pointer, slice) = match self.previous.take() {
            Some(prev) => prev,
            None => loop {
                match self.socket.next().await {
                    Some(Ok(Message::Binary(msg))) => {
                        // Payloads of large frames are referenced, small ones copied.
                        let slice = msg.len() > COPY_SIZE;
                        break (msg, 0, slice);
                    }
                    Some(Ok(Message::Close(_))) => return Err(None),
                    Some(Ok(Message::Text(_))) => {
                        return Err(Some("Received text message, expected binary".to_string()));
                    }
                    // Keep-alive frames carry no data; tungstenite answers pings itself.
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Frame(_))) => {
                        return Err(Some("Received frame message, expected binary".to_string()));
                    }
                    Some(Err(e)) => {
                        return Err(Some(format!("Reading message from client failed: {:?}", e)));
                    }
                    None => return Err(Some("Connection was closed by the client".to_string())),
                }
            },
        };

        let (header, size) = ClientHeader::deserialize(&data[pointer..])
            .map_err(|_| Some("Failed to deserialize message header".to_string()))?;
        match header {
            ClientHeader::Value(id, type_id, signal, data_size) => {
                let all_size = size + data_size as usize;
                if all_size > data.len() - pointer {
                    return Err(Some("Incomplete data received".to_string()));
                }
                let header_data = if slice {
                    data.slice(pointer + size..pointer + all_size)
                } else {
                    Bytes::copy_from_slice(&data[pointer + size..pointer + all_size])
                };
                if pointer + all_size < data.len() {
                    self.previous = Some((data, pointer + all_size, slice));
                }
                Ok(ClientMessage::Value(id, type_id, signal, header_data))
            }
            ClientHeader::Signal(id, type_id, data_size) => {
                let all_size = size + data_size as usize;
                if all_size > data.len() - pointer {
                    return Err(Some("Incomplete data received".to_string()));
                }
                let header_data = if slice {
                    data.slice(pointer + size..pointer + all_size)
                } else {
                    Bytes::copy_from_slice(&data[pointer + size..pointer + all_size])
                };
                if pointer + all_size < data.len() {
                    self.previous = Some((data, pointer + all_size, slice));
                }
                Ok(ClientMessage::Signal(id, type_id, header_data))
            }
            ClientHeader::Ack(id) => {
                if pointer + size < data.len() {
                    self.previous = Some((data, pointer + size, slice));
                }
                Ok(ClientMessage::Ack(id))
            }
            ClientHeader::Message(data_size) => {
                let all_size = size + data_size as usize;
                if all_size > data.len() - pointer {
                    return Err(Some("Incomplete data received".to_string()));
                }
                let message_data = if slice {
                    data.slice(pointer + size..pointer + all_size)
                } else {
                    Bytes::copy_from_slice(&data[pointer + size..pointer + all_size])
                };
                if pointer + all_size < data.len() {
                    self.previous = Some((data, pointer + all_size, slice));
                }
                Ok(ClientMessage::Message(message_data))
            }
            ClientHeader::Handshake(protocol_version, client_version, hash) => {
                if pointer + size < data.len() {
                    self.previous = Some((data, pointer + size, slice));
                }
                Ok(ClientMessage::Handshake(
                    protocol_version,
                    client_version,
                    hash,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::SinkExt;

    #[test]
    fn socket_reader_rejects_truncation_and_malformed_later_messages_then_recovers() {
        if crate::test_support::isolated() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let addr = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let stream = TcpStream::connect(addr).await.unwrap();
                let (mut ws, _) = tokio_tungstenite::client_async(format!("ws://{addr}"), stream)
                    .await
                    .unwrap();
                let mut truncated =
                    postcard::to_stdvec(&ClientHeader::Value(7, 3, true, 5)).unwrap();
                truncated.push(1);
                let mut later = postcard::to_stdvec(&ClientHeader::Ack(31)).unwrap();
                later.push(255); // no complete enum discriminant
                for bytes in [
                    vec![255],
                    truncated,
                    later,
                    postcard::to_stdvec(&ClientHeader::Ack(73)).unwrap(),
                ] {
                    ws.send(Message::Binary(bytes.into())).await.unwrap();
                }
                ws.close(None).await.unwrap();
            });
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (_writer, reader) = ws.split();
            let mut reader = SocketReader::new(reader);
            assert!(matches!(reader.next().await, Err(Some(error)) if error.contains("header")));
            assert!(
                matches!(reader.next().await, Err(Some(error)) if error.contains("Incomplete data"))
            );
            assert!(matches!(reader.next().await, Ok(ClientMessage::Ack(31))));
            assert!(matches!(reader.next().await, Err(Some(error)) if error.contains("header")));
            assert!(matches!(reader.next().await, Ok(ClientMessage::Ack(73))));
            peer.await.unwrap();
        });
    }

    #[test]
    fn socket_reader_skips_keep_alive_frames() {
        if crate::test_support::isolated() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let addr = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let stream = TcpStream::connect(addr).await.unwrap();
                let (mut ws, _) = tokio_tungstenite::client_async(format!("ws://{addr}"), stream)
                    .await
                    .unwrap();
                let ack = postcard::to_stdvec(&ClientHeader::Ack(31)).unwrap();
                for message in [
                    Message::Ping(Bytes::from_static(b"ping")),
                    Message::Pong(Bytes::new()),
                    Message::Binary(ack.into()),
                ] {
                    ws.send(message).await.unwrap();
                }
                ws.close(None).await.unwrap();
            });
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (_writer, reader) = ws.split();
            let mut reader = SocketReader::new(reader);
            assert!(matches!(reader.next().await, Ok(ClientMessage::Ack(31))));
            assert!(matches!(reader.next().await, Err(None)));
            peer.await.unwrap();
        });
    }
}
