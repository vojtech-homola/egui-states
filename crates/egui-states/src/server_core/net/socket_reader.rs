use std::sync::Arc;

use bytes::Bytes;

use crate::protocol::ClientHeader;
use crate::server_core::net::heartbeat::Activity;
use crate::server_core::net::transport::{FrameSource, Incoming};

const COPY_SIZE: usize = 1024; // 1 KB

pub(crate) enum ClientMessage {
    Value(u64, u32, bool, Bytes),
    Signal(u64, u32, Bytes),
    Ack(u64),
    Message(Bytes),
    Handshake(u16, Option<u64>, Option<String>),
}

pub(crate) struct SocketReader<S> {
    socket: S,
    previous: Option<(Bytes, usize, bool)>,
    activity: Arc<Activity>,
}

impl<S: FrameSource> SocketReader<S> {
    pub(crate) fn new(socket: S) -> Self {
        Self {
            socket,
            previous: None,
            activity: Activity::new(),
        }
    }

    /// Returns the record of when the client last sent any frame.
    pub(crate) fn activity(&self) -> Arc<Activity> {
        self.activity.clone()
    }

    pub(crate) async fn next(&mut self) -> Result<ClientMessage, Option<String>> {
        let (data, pointer, slice) = match self.previous.take() {
            Some(prev) => prev,
            None => loop {
                let frame = self.socket.next_frame().await;
                // Every frame, including the pongs answering heartbeat pings,
                // shows that the client is still there.
                if let Some(Ok(_)) = frame {
                    self.activity.touch();
                }
                match frame {
                    Some(Ok(Incoming::Binary(msg))) => {
                        // Payloads of large frames are referenced, small ones copied.
                        let slice = msg.len() > COPY_SIZE;
                        break (msg, 0, slice);
                    }
                    Some(Ok(Incoming::Close)) => return Err(None),
                    // Keep-alive frames carry no data.
                    Some(Ok(Incoming::KeepAlive)) => continue,
                    Some(Err(e)) => return Err(Some(e)),
                    None => return Err(Some("Connection was closed by the client".to_string())),
                }
            },
        };

        let (header, size) = ClientHeader::deserialize(&data[pointer..])
            .map_err(|_| Some("Failed to deserialize message header".to_string()))?;
        let start = pointer + size;
        let end = start
            .checked_add(header.payload_len() as usize)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| Some("Incomplete data received".to_string()))?;
        let payload = if slice {
            data.slice(start..end)
        } else {
            Bytes::copy_from_slice(&data[start..end])
        };
        if end < data.len() {
            self.previous = Some((data, end, slice));
        }

        Ok(match header {
            ClientHeader::Value(id, type_id, signal, _) => {
                ClientMessage::Value(id, type_id, signal, payload)
            }
            ClientHeader::Signal(id, type_id, _) => ClientMessage::Signal(id, type_id, payload),
            ClientHeader::Ack(id) => ClientMessage::Ack(id),
            ClientHeader::Message(_) => ClientMessage::Message(payload),
            ClientHeader::Handshake(protocol_version, client_version, hash) => {
                ClientMessage::Handshake(protocol_version, client_version, hash)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpStream;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;

    #[test]
    fn socket_reader_rejects_truncation_and_malformed_later_messages_then_recovers() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (client, opened) = crate::server_core::net::transport::memory::pair().await;
            let mut truncated = postcard::to_stdvec(&ClientHeader::Value(7, 3, true, 5)).unwrap();
            truncated.push(1);
            let mut later = postcard::to_stdvec(&ClientHeader::Ack(31)).unwrap();
            later.push(255); // no complete enum discriminant
            for bytes in [
                vec![255],
                truncated,
                later,
                postcard::to_stdvec(&ClientHeader::Ack(73)).unwrap(),
            ] {
                client.send.send(Incoming::Binary(bytes.into())).unwrap();
            }
            drop(client);

            let mut reader = SocketReader::new(opened.source);
            assert!(matches!(reader.next().await, Err(Some(error)) if error.contains("header")));
            assert!(
                matches!(reader.next().await, Err(Some(error)) if error.contains("Incomplete data"))
            );
            assert!(matches!(reader.next().await, Ok(ClientMessage::Ack(31))));
            assert!(matches!(reader.next().await, Err(Some(error)) if error.contains("header")));
            assert!(matches!(reader.next().await, Ok(ClientMessage::Ack(73))));
            assert!(
                matches!(reader.next().await, Err(Some(error)) if error.contains("closed by the client"))
            );
        });
    }

    /// Over a WebSocket, pings and pongs reach the reader as keep-alive
    /// frames without messages.
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
