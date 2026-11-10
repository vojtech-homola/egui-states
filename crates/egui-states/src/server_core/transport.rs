//! The frames a connection exchanges, apart from how they travel.
//!
//! The run loop takes connections from a [`Listener`] and reads and writes
//! their frames through [`FrameSource`] and [`FrameSink`], so the session code
//! does not depend on WebSockets over TCP. Tests drive it in memory.

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, timeout_at};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use crate::serialization::WEBSOCKET_MAX_SIZE;
use crate::server_core::origin::OriginPolicy;
use crate::server_core::signals::SignalsManager;
use crate::server_core::web::{self, Dispatch, WebSite};

/// A frame from the client.
pub(crate) enum Incoming {
    /// A frame of client messages.
    Binary(Bytes),
    /// A frame without messages, such as a pong. It still shows that the
    /// client is there.
    KeepAlive,
    /// The client closed the connection.
    Close,
}

/// A frame to the client.
pub(crate) enum Outgoing {
    /// A frame of server messages.
    Binary(Bytes),
    /// A heartbeat ping.
    Ping,
}

/// The frames a client sends.
pub(crate) trait FrameSource: Send + 'static {
    /// Waits for the next frame. An error describes a failed read or a frame
    /// the protocol does not use; `None` means the connection is gone.
    fn next_frame(&mut self) -> impl Future<Output = Option<Result<Incoming, String>>> + Send;
}

/// The frames sent to a client.
pub(crate) trait FrameSink: Send + 'static {
    /// Sends `frame`; the error describes why it could not be sent.
    fn send_frame(&mut self, frame: Outgoing) -> impl Future<Output = Result<(), String>> + Send;

    /// Closes the connection, ignoring failures.
    fn close(&mut self) -> impl Future<Output = ()> + Send;
}

/// The frame channel of a connection that has been opened.
pub(crate) struct Opened<L: Listener + ?Sized> {
    pub(crate) sink: L::Sink,
    pub(crate) source: L::Source,
    pub(crate) peer_addr: String,
}

/// Where the run loop takes its connections from.
pub(crate) trait Listener: Send + 'static {
    /// A connection that has been accepted but not opened yet.
    type Connection: Send + 'static;
    type Source: FrameSource;
    type Sink: FrameSink;

    /// Waits for the next connection. The error describes a failed accept,
    /// after which the run loop pauses briefly.
    fn accept(&mut self) -> impl Future<Output = Result<Self::Connection, String>> + Send;

    /// While `site` is served, answers plain HTTP requests on `connection`
    /// until `deadline`. Returns the connection when it asks to be opened.
    fn serve_web(
        connection: Self::Connection,
        site: Option<Arc<WebSite>>,
        deadline: Instant,
        signals: &SignalsManager,
    ) -> impl Future<Output = Option<Self::Connection>> + Send;

    /// Opens the frame channel of `connection`, refusing pages `origins` does
    /// not allow.
    fn open(
        connection: Self::Connection,
        origins: &OriginPolicy,
        signals: &SignalsManager,
    ) -> impl Future<Output = Option<Opened<Self>>> + Send;
}

type WebSocket = WebSocketStream<TcpStream>;

impl Listener for TcpListener {
    type Connection = TcpStream;
    type Source = SplitStream<WebSocket>;
    type Sink = SplitSink<WebSocket, Message>;

    async fn accept(&mut self) -> Result<TcpStream, String> {
        let (stream, _) = TcpListener::accept(self)
            .await
            .map_err(|e| format!("accepting connection failed: {:?}", e))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("failed to set TCP_NODELAY: {:?}", e))?;
        Ok(stream)
    }

    async fn serve_web(
        stream: TcpStream,
        site: Option<Arc<WebSite>>,
        deadline: Instant,
        signals: &SignalsManager,
    ) -> Option<TcpStream> {
        let Some(site) = site else {
            return Some(stream);
        };
        match timeout_at(deadline, web::dispatch(stream)).await {
            Ok(Ok(Dispatch::WebSocket(stream))) => Some(stream),
            Ok(Ok(Dispatch::Http(stream, request))) => {
                // Browsers abort downloads on navigation, so failures here are
                // not server errors.
                if let Err(_e) = web::serve(stream, request, &site).await {
                    #[cfg(debug_assertions)]
                    signals.debug(format!("serving web page failed: {:?}", _e));
                }
                None
            }
            // Browsers open speculative connections that may stay idle and be
            // closed unused, so these are not reported.
            Ok(Ok(Dispatch::Closed)) | Err(_) => None,
            Ok(Err(e)) => {
                signals.error(format!("reading request failed: {:?}", e));
                None
            }
        }
    }

    async fn open(
        stream: TcpStream,
        origins: &OriginPolicy,
        signals: &SignalsManager,
    ) -> Option<Opened<Self>> {
        let mut websocket_config = WebSocketConfig::default();
        websocket_config.max_message_size = Some(WEBSOCKET_MAX_SIZE);
        websocket_config.max_frame_size = Some(WEBSOCKET_MAX_SIZE);
        // Refuse the upgrade of a page that may not connect before the protocol
        // handshake, so it never reaches the token check or the current client.
        let mut rejection = None;
        // tungstenite fixes the callback's error type.
        #[allow(clippy::result_large_err)]
        let check_origin = |request: &Request, response: Response| {
            origins
                .check(request.headers())
                .map(|()| response)
                .map_err(|reason| {
                    rejection = Some(reason);
                    let mut forbidden = ErrorResponse::new(Some(
                        "This page may not connect to the state server.\n".to_string(),
                    ));
                    *forbidden.status_mut() = StatusCode::FORBIDDEN;
                    forbidden
                })
        };
        let accepted = tokio_tungstenite::accept_hdr_async_with_config(
            stream,
            check_origin,
            Some(websocket_config),
        )
        .await;
        let websocket = match accepted {
            Ok(ws) => ws,
            Err(e) => {
                match rejection {
                    Some(reason) => signals.warning(format!("rejected connection: {reason}")),
                    None => signals.error(format!("websocket handshake failed: {:?}", e)),
                }
                return None;
            }
        };

        let peer_addr = websocket
            .get_ref()
            .peer_addr()
            .map_or("".to_string(), |addr| addr.to_string());
        let (sink, source) = websocket.split();
        Some(Opened {
            sink,
            source,
            peer_addr,
        })
    }
}

impl FrameSource for SplitStream<WebSocket> {
    async fn next_frame(&mut self) -> Option<Result<Incoming, String>> {
        let frame = match StreamExt::next(self).await? {
            Ok(frame) => frame,
            Err(e) => return Some(Err(format!("Reading message from client failed: {:?}", e))),
        };
        Some(match frame {
            Message::Binary(data) => Ok(Incoming::Binary(data)),
            // tungstenite answers pings itself.
            Message::Ping(_) | Message::Pong(_) => Ok(Incoming::KeepAlive),
            Message::Close(_) => Ok(Incoming::Close),
            Message::Text(_) => Err("Received text message, expected binary".to_string()),
            Message::Frame(_) => Err("Received frame message, expected binary".to_string()),
        })
    }
}

impl FrameSink for SplitSink<WebSocket, Message> {
    async fn send_frame(&mut self, frame: Outgoing) -> Result<(), String> {
        let message = match frame {
            Outgoing::Binary(data) => Message::Binary(data),
            Outgoing::Ping => Message::Ping(Bytes::new()),
        };
        SinkExt::send(self, message)
            .await
            .map_err(|e| format!("{:?}", e))
    }

    async fn close(&mut self) {
        let _ = SinkExt::close(self).await;
    }
}

/// Connections that travel through channels, for tests of the run loop.
#[cfg(test)]
pub(crate) mod memory {
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::*;

    /// Accepts the connections made with [`MemoryConnector::connect`].
    pub(crate) struct MemoryListener(UnboundedReceiver<MemoryConnection>);

    /// Makes connections to a [`MemoryListener`].
    #[derive(Clone)]
    pub(crate) struct MemoryConnector(UnboundedSender<MemoryConnection>);

    pub(crate) struct MemoryConnection {
        sink: MemorySink,
        source: MemorySource,
    }

    pub(crate) struct MemorySource(UnboundedReceiver<Incoming>);

    pub(crate) struct MemorySink(UnboundedSender<Outgoing>);

    /// The client end of an in-memory connection. Dropping it closes the
    /// connection as a lost socket would.
    pub(crate) struct MemoryClient {
        pub(crate) frames: UnboundedReceiver<Outgoing>,
        pub(crate) send: UnboundedSender<Incoming>,
    }

    pub(crate) fn listener() -> (MemoryListener, MemoryConnector) {
        let (connector, listener) = unbounded_channel();
        (MemoryListener(listener), MemoryConnector(connector))
    }

    /// One connection, opened as the run loop opens it.
    pub(crate) async fn pair() -> (MemoryClient, Opened<MemoryListener>) {
        let (mut listener, connector) = listener();
        let client = connector.connect().unwrap();
        let connection = listener.accept().await.unwrap();
        let opened =
            MemoryListener::open(connection, &OriginPolicy::default(), &SignalsManager::new())
                .await
                .unwrap();
        (client, opened)
    }

    impl MemoryConnector {
        /// Opens a connection; `None` once the listener is gone.
        pub(crate) fn connect(&self) -> Option<MemoryClient> {
            let (to_server, from_client) = unbounded_channel();
            let (to_client, from_server) = unbounded_channel();
            let connection = MemoryConnection {
                sink: MemorySink(to_client),
                source: MemorySource(from_client),
            };
            self.0.send(connection).ok()?;
            Some(MemoryClient {
                frames: from_server,
                send: to_server,
            })
        }
    }

    impl Listener for MemoryListener {
        type Connection = MemoryConnection;
        type Source = MemorySource;
        type Sink = MemorySink;

        async fn accept(&mut self) -> Result<MemoryConnection, String> {
            match self.0.recv().await {
                Some(connection) => Ok(connection),
                // No connector is left, so no connection will come.
                None => std::future::pending().await,
            }
        }

        async fn serve_web(
            connection: MemoryConnection,
            _site: Option<Arc<WebSite>>,
            _deadline: Instant,
            _signals: &SignalsManager,
        ) -> Option<MemoryConnection> {
            Some(connection)
        }

        async fn open(
            connection: MemoryConnection,
            _origins: &OriginPolicy,
            _signals: &SignalsManager,
        ) -> Option<Opened<Self>> {
            Some(Opened {
                sink: connection.sink,
                source: connection.source,
                peer_addr: "memory".to_string(),
            })
        }
    }

    impl FrameSource for MemorySource {
        async fn next_frame(&mut self) -> Option<Result<Incoming, String>> {
            self.0.recv().await.map(Ok)
        }
    }

    impl FrameSink for MemorySink {
        async fn send_frame(&mut self, frame: Outgoing) -> Result<(), String> {
            self.0
                .send(frame)
                .map_err(|_| "the client is gone".to_string())
        }

        async fn close(&mut self) {}
    }
}
