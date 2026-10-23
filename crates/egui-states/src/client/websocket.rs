use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use std::net::SocketAddrV4;
use tokio::net::TcpStream;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::client::messages::{MessagesParser, ServerMessage};
use crate::serialization::{FastVec, WEBSOCKET_MAX_SIZE};

pub(crate) async fn build_ws(address: SocketAddrV4) -> Result<(WsClientRead, WsClientSend), ()> {
    let address = format!("ws://{}/ws", address);
    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(WEBSOCKET_MAX_SIZE);
    websocket_config.max_frame_size = Some(WEBSOCKET_MAX_SIZE);
    let res = connect_async_with_config(&address, Some(websocket_config), false).await;

    if res.is_err() {
        #[cfg(debug_assertions)]
        println!(
            "connecting to server at {:?} failed: {:?}",
            address,
            res.err()
        );
        return Err(());
    }

    // get the socket
    let socket = res.unwrap().0;

    // split the socket
    let (socket_write, socket_read) = socket.split();

    Ok((
        WsClientRead {
            stream: socket_read,
            parser: MessagesParser::empty(),
        },
        WsClientSend { sink: socket_write },
    ))
}

pub(crate) struct WsClientRead {
    stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    parser: MessagesParser,
}

impl WsClientRead {
    pub(crate) async fn read(&mut self) -> Result<ServerMessage, &'static str> {
        if let Some(message) = self.parser.next()? {
            return Ok(message);
        }

        let message = loop {
            match self.stream.next().await {
                // Keep-alive frames carry no data; tungstenite answers pings itself.
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                message => break message,
            }
        };

        match message {
            Some(message) => match message {
                Ok(message) => match message {
                    Message::Binary(data) => {
                        let (parser, message) = MessagesParser::from_bytes(data)?;
                        self.parser = parser;
                        Ok(message)
                    }
                    Message::Close(_) => Err("Connection closed by server"),
                    _ => {
                        #[cfg(debug_assertions)]
                        println!("Unexpected message from server: {:?}", message);
                        Err("Unexpected message from server")
                    }
                },
                Err(e) => {
                    #[cfg(debug_assertions)]
                    println!("Reading message from server failed: {:?}", e);
                    let _ = e;
                    Err("Reading message from server failed")
                }
            },
            None => {
                #[cfg(debug_assertions)]
                println!("Connection closed by server");
                Err("Connection closed by server")
            }
        }
    }
}

pub(crate) struct WsClientSend {
    sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
}

impl WsClientSend {
    #[inline]
    pub(crate) async fn close(&mut self) {
        let _ = self.sink.close().await;
    }

    pub(crate) async fn send(&mut self, data: FastVec<64>) -> Result<(), ()> {
        let message = Message::Binary(data.to_bytes());
        match self.sink.send(message).await {
            Ok(_) => Ok(()),
            Err(_e) => {
                #[cfg(debug_assertions)]
                println!("Sending message to socket failed: {:?}", _e);
                Err(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::{ServerHeader, serialize};

    #[test]
    fn client_reader_skips_keep_alive_frames() {
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
            let std::net::SocketAddr::V4(addr) = listener.local_addr().unwrap() else {
                unreachable!("bound to an IPv4 address");
            };
            let peer = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let update = serialize::<_, 32>(&ServerHeader::Update(0.5)).unwrap();
                for message in [
                    Message::Ping(bytes::Bytes::from_static(b"ping")),
                    Message::Pong(bytes::Bytes::new()),
                    Message::Binary(update.to_bytes()),
                ] {
                    ws.send(message).await.unwrap();
                }
                ws.close(None).await.unwrap();
            });
            let (mut reader, _writer) = build_ws(addr).await.unwrap();
            assert!(matches!(reader.read().await, Ok(ServerMessage::Update(dt)) if dt == 0.5));
            assert!(reader.read().await.is_err());
            peer.await.unwrap();
        });
    }
}
