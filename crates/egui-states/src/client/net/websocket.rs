use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use tokio::net::TcpStream;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::client::messages::{ReadError, ServerFrames, ServerReader};
use crate::protocol::{FastVec, WEBSOCKET_MAX_SIZE};

pub(crate) async fn build_ws(address: &str) -> Result<(WsClientRead, WsClientSend), String> {
    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(WEBSOCKET_MAX_SIZE);
    websocket_config.max_frame_size = Some(WEBSOCKET_MAX_SIZE);
    // Disable Nagle's algorithm, as the server does for its sockets: the
    // server answers value changes and acknowledgements with nothing, so its
    // delayed ACK would otherwise hold back each next small client message.
    let disable_nagle = true;
    let res = connect_async_with_config(address, Some(websocket_config), disable_nagle).await;

    let socket = match res {
        Ok(res) => res.0,
        Err(e) => return Err(format!("Connecting to server at {} failed: {}", address, e)),
    };

    // split the socket
    let (socket_write, socket_read) = socket.split();

    Ok((
        ServerReader::new(WsFrames(socket_read)),
        WsClientSend { sink: socket_write },
    ))
}

pub(crate) type WsClientRead = ServerReader<WsFrames>;

pub(crate) struct WsFrames(SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>);

impl ServerFrames for WsFrames {
    async fn next_frame(&mut self) -> Result<Bytes, ReadError> {
        let message = loop {
            match self.0.next().await {
                // Keep-alive frames carry no data; tungstenite answers pings itself.
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                message => break message,
            }
        };

        match message {
            Some(Ok(Message::Binary(data))) => Ok(data),
            Some(Ok(Message::Close(frame))) => Err(ReadError::Closed(
                frame.map(|frame| (frame.code.into(), frame.reason.to_string())),
            )),
            Some(Ok(_message)) => {
                #[cfg(debug_assertions)]
                println!("Unexpected message from server: {:?}", _message);
                Err(ReadError::Failed("Unexpected message from server"))
            }
            Some(Err(_e)) => {
                #[cfg(debug_assertions)]
                println!("Reading message from server failed: {:?}", _e);
                Err(ReadError::Failed("Reading message from server failed"))
            }
            None => {
                #[cfg(debug_assertions)]
                println!("Connection closed by server");
                Err(ReadError::Closed(None))
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

    /// Sends `data`; the error describes why it could not be sent.
    pub(crate) async fn send(&mut self, data: FastVec<64>) -> Result<(), String> {
        let message = Message::Binary(data.into_bytes());
        self.sink.send(message).await.map_err(|e| {
            #[cfg(debug_assertions)]
            println!("Sending message to socket failed: {:?}", e);
            e.to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ServerHeader, serialize};

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
                    Message::Binary(update.into_bytes()),
                ] {
                    ws.send(message).await.unwrap();
                }
                ws.close(None).await.unwrap();
            });
            let (mut reader, _writer) = build_ws(&format!("ws://{addr}/ws")).await.unwrap();
            assert!(matches!(reader.read().await, Ok((ServerHeader::Update(dt), _)) if dt == 0.5));
            assert!(reader.read().await.is_err());
            peer.await.unwrap();
        });
    }

    /// The code and reason of the server's close frame reach the reader.
    #[test]
    fn client_reader_reports_the_close_code_and_reason() {
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
                use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let frame = CloseFrame {
                    code: 4003.into(),
                    reason: "busy".into(),
                };
                ws.close(Some(frame)).await.unwrap();
            });
            let (mut reader, _writer) = build_ws(&format!("ws://{addr}/ws")).await.unwrap();
            let Err(error) = reader.read().await else {
                panic!("the reader returned a message");
            };
            assert_eq!(error, ReadError::Closed(Some((4003, "busy".to_string()))));
            peer.await.unwrap();
        });
    }
}
