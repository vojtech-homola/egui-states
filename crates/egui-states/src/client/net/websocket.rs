use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message, protocol::WebSocketConfig};

use crate::client::messages::{ReadError, ServerFrames, ServerReader};
use crate::client::net::Target;
use crate::protocol::{FastVec, WEBSOCKET_MAX_SIZE};

pub(crate) async fn build_ws(target: &Target) -> Result<(WsClientRead, WsClientSend), String> {
    let failed =
        |reason: String| format!("Connecting to server at {} failed: {reason}", target.url);
    // Prepare TLS first, so an invalid fingerprint fails before connecting.
    #[cfg(feature = "tls")]
    let tls = target
        .tls
        .as_ref()
        .map(|tls| super::tls::connector(tls, &target.host))
        .transpose()
        .map_err(failed)?;

    let stream = TcpStream::connect((target.host.as_str(), target.port))
        .await
        .map_err(|error| failed(error.to_string()))?;
    // Disable Nagle's algorithm, as the server does for its sockets: the
    // server answers value changes and acknowledgements with nothing, so its
    // delayed ACK would otherwise hold back each next small client message.
    stream
        .set_nodelay(true)
        .map_err(|error| failed(error.to_string()))?;
    #[cfg(feature = "tls")]
    let stream = match tls {
        None => ClientStream::Plain(stream),
        Some((connector, name)) => {
            let stream = connector
                .connect(name, stream)
                .await
                .map_err(|error| failed(super::tls::describe(&error)))?;
            ClientStream::Tls(Box::new(stream))
        }
    };
    #[cfg(not(feature = "tls"))]
    let stream = ClientStream::Plain(stream);

    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(WEBSOCKET_MAX_SIZE);
    websocket_config.max_frame_size = Some(WEBSOCKET_MAX_SIZE);
    let socket =
        tokio_tungstenite::client_async_with_config(&target.url, stream, Some(websocket_config))
            .await
            .map_err(|error| failed(describe(error)))?
            .0;

    // split the socket
    let (socket_write, socket_read) = socket.split();

    Ok((
        ServerReader::new(WsFrames(socket_read)),
        WsClientSend { sink: socket_write },
    ))
}

/// Explains why the WebSocket handshake failed.
fn describe(error: WsError) -> String {
    match error {
        // The answer of a server that speaks TLS to a plain request.
        WsError::Http(response) if response.status() == 426 => {
            "the server requires TLS; connect with `ClientBuilder::tls`".to_string()
        }
        error => error.to_string(),
    }
}

/// The connection to the server, plain or encrypted.
enum ClientStream {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for ClientStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ClientStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Plain(stream) => stream.is_write_vectored(),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => stream.is_write_vectored(),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_shutdown(cx),
        }
    }
}

pub(crate) type WsClientRead = ServerReader<WsFrames>;

pub(crate) struct WsFrames(SplitStream<WebSocketStream<ClientStream>>);

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
    sink: SplitSink<WebSocketStream<ClientStream>, Message>,
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
            let target = Target {
                url: format!("ws://{addr}/ws"),
                host: addr.ip().to_string(),
                port: addr.port(),
                #[cfg(feature = "tls")]
                tls: None,
            };
            let (mut reader, _writer) = build_ws(&target).await.unwrap();
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
            let target = Target {
                url: format!("ws://{addr}/ws"),
                host: addr.ip().to_string(),
                port: addr.port(),
                #[cfg(feature = "tls")]
                tls: None,
            };
            let (mut reader, _writer) = build_ws(&target).await.unwrap();
            let Err(error) = reader.read().await else {
                panic!("the reader returned a message");
            };
            assert_eq!(error, ReadError::Closed(Some((4003, "busy".to_string()))));
            peer.await.unwrap();
        });
    }
}
