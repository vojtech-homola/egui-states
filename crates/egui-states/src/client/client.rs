use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use egui::Context;
use parking_lot::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::PROTOCOL_VERSION;
use crate::State;
use crate::client::messages::{ChannelMessage, MessageSender, MessagesSerializer, handle_message};
use crate::client::states_creator::{StatesCreatorClient, ValuesList};
use crate::event::Event;
use crate::serialization::ClientHeader;

#[cfg(not(target_arch = "wasm32"))]
use crate::client::websocket::build_ws;

#[cfg(target_arch = "wasm32")]
use crate::client::websocket_wasm::build_ws;

async fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    mut rx: UnboundedReceiver<Option<ChannelMessage>>,
    sender: MessageSender,
    client: Client,
    version: Option<u64>,
    hash: Option<String>,
) {
    loop {
        // wait for the connection signal
        client.wait_connection().await;
        client.set_state(ConnectionState::NotConnected);

        // try to connect to the server
        let res = build_ws(addr).await;
        if res.is_err() {
            continue;
        }
        let (mut socket_read, mut socket_send) = res.unwrap();

        // clean message queue before starting
        while !rx.is_empty() {
            let _ = rx.recv().await;
        }

        // communicate handshake and initialization -------------------------
        let message = ClientHeader::serialize_handshake(PROTOCOL_VERSION, version, hash.clone());
        if let Err(_) = socket_send.send(message).await {
            #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
            println!("Sending handshake failed.");
            #[cfg(all(debug_assertions, target_arch = "wasm32"))]
            log::error!("Sending handshake failed.");
            continue;
        }

        // read -----------------------------------------
        let th_vals = vals.clone();
        let th_client = client.clone();
        let th_sender = sender.clone();

        let recv_future = async move {
            loop {
                // read the message
                match socket_read.read().await {
                    Ok(msg) => {
                        if let Err(e) = handle_message(msg, &th_vals, &th_client).await {
                            let error = format!("handling message from server failed: {:?}", e);
                            th_sender.send_message(&error);
                            print_error(&error);
                            // break; TODO: decide if we want to break the loop on error
                        }
                    }
                    Err(e) => {
                        print_error(&format!("Connection with server failed: {:?}", e));
                        break;
                    }
                }
            }
            th_sender.close();
        };

        #[cfg(not(target_arch = "wasm32"))]
        let recv_future = tokio::spawn(recv_future);

        // send -----------------------------------------
        let send_future = async move {
            let mut serializer = MessagesSerializer::new(rx);

            while let Some(message) = serializer.next().await {
                if let Err(_) = socket_send.send(message).await {
                    break;
                }
            }

            socket_send.close().await;
            serializer.close()
        };

        #[cfg(not(target_arch = "wasm32"))]
        let send_future = tokio::spawn(send_future);

        client.set_state(ConnectionState::Connected);

        #[cfg(not(target_arch = "wasm32"))]
        {
            // wait for the read thread to finish
            let _ = recv_future.await;

            // wait for the send thread
            rx = send_future.await.unwrap();
        }

        #[cfg(target_arch = "wasm32")]
        {
            let (_, rx_) = tokio::join!(recv_future, send_future);
            rx = rx_;
        }

        client.set_state(ConnectionState::Disconnected);
    }
}

pub(crate) fn print_error(error: &str) {
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    println!("{}", error);
    #[cfg(all(debug_assertions, target_arch = "wasm32"))]
    log::error!("{}", error);
    let _ = error;
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Current client connection status.
pub enum ConnectionState {
    /// No connection has been established or a connection attempt is pending.
    NotConnected,
    /// The WebSocket handshake completed and state synchronization is active.
    Connected,
    /// A previously active connection ended.
    Disconnected,
}

struct ClientInner {
    context: Option<Context>,
    connect_signal: Event,
    state: Arc<RwLock<ConnectionState>>,
    sender: MessageSender,
}

impl ClientInner {
    fn set_context(&mut self, context: Context) {
        self.context.replace(context);
    }
}

#[derive(Clone)]
/// Handle for controlling the background state-synchronization client.
pub struct Client(Arc<ClientInner>);

impl Client {
    pub(crate) fn new(context: Option<Context>, sender: MessageSender) -> Self {
        let inner = ClientInner {
            context,
            connect_signal: Event::new(),
            state: Arc::new(RwLock::new(ConnectionState::NotConnected)),
            sender,
        };

        Self(Arc::new(inner))
    }

    /// Sets the egui context used to request repaints after incoming updates.
    ///
    /// This must be called before cloning the client; use
    /// [`ClientBuilder::context`] when possible.
    pub fn set_context(&mut self, context: Context) {
        Arc::get_mut(&mut self.0).unwrap().set_context(context);
    }

    /// Requests an egui repaint immediately, or after `time` seconds if positive.
    pub fn update(&self, time: f32) {
        if let Some(ctx) = &self.0.context {
            if time > 0.0 {
                ctx.request_repaint_after(Duration::from_secs_f32(time));
            } else {
                ctx.request_repaint();
            }
        }
    }

    pub(crate) async fn wait_connection(&self) {
        self.0.connect_signal.clear();
        self.0.connect_signal.wait_clear_async().await;
    }

    /// Starts or retries a connection to the configured server.
    pub fn connect(&self) {
        self.0.connect_signal.set();
    }

    /// Closes the active connection.
    pub fn disconnect(&self) {
        self.0.sender.close();
    }

    pub(crate) fn set_state(&self, state: ConnectionState) {
        *self.0.state.write() = state;
        if let Some(ctx) = &self.0.context {
            ctx.request_repaint();
        }
    }

    /// Returns the current connection status.
    pub fn get_state(&self) -> ConnectionState {
        *self.0.state.read()
    }
}

/// Configures and builds a typed client and its root state tree.
pub struct ClientBuilder<T> {
    creator: StatesCreatorClient,
    states: T,
    sender: MessageSender,
    rx: UnboundedReceiver<Option<ChannelMessage>>,
    addr: Ipv4Addr,
    context: Option<Context>,
    version: Option<u64>,
    token: Option<String>,
}

impl<T> ClientBuilder<T>
where
    T: State,
{
    /// Creates a builder targeting `127.0.0.1` with no authentication settings.
    pub fn new() -> Self {
        let (sender, rx) = MessageSender::new();

        let mut creator = StatesCreatorClient::new(sender.clone(), "root".to_string());
        let states = T::new(&mut creator);
        let addr = Ipv4Addr::new(127, 0, 0, 1);

        Self {
            creator,
            states,
            sender,
            rx,
            addr,
            context: None,
            version: None,
            token: None,
        }
    }

    /// Sets the server IPv4 address.
    pub fn addr(self, addr: Ipv4Addr) -> Self {
        Self { addr, ..self }
    }

    /// Requires the server to accept this application version.
    pub fn version(self, version: u64) -> Self {
        Self {
            version: Some(version),
            ..self
        }
    }

    /// Sets the authentication token sent during the handshake.
    pub fn token(self, token: String) -> Self {
        Self {
            token: Some(token),
            ..self
        }
    }

    /// Sets the egui context used for repaint requests.
    pub fn context(self, context: Context) -> Self {
        Self {
            context: Some(context),
            ..self
        }
    }

    /// Returns the stable hash of the configured state tree.
    ///
    /// Generated server bindings expose the same hash so mismatched state
    /// layouts can be rejected during the handshake.
    pub fn get_version_hash(&self) -> u64 {
        self.creator.get_version_hash()
    }

    /// Builds the root state and starts the background client task for `port`.
    ///
    /// The task waits until [`Client::connect`] is called before opening a
    /// connection. On native targets it owns a dedicated Tokio runtime; on WASM
    /// it is spawned on the current browser executor.
    pub fn build(self, port: u16) -> (T, Client) {
        let Self {
            creator,
            states,
            sender,
            rx,
            addr,
            context,
            version,
            token,
        } = self;

        let addr = SocketAddrV4::new(addr, port);
        let values = creator.get_values();
        let client = Client::new(context, sender.clone());
        let client_out = client.clone();

        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::thread;
            use tokio::runtime::Builder;

            let runtime = Builder::new_multi_thread()
                .thread_name("Client Runtime")
                .enable_io()
                .worker_threads(2)
                .build()
                .unwrap();

            let thread = thread::Builder::new().name("Client".to_string());

            let _ = thread.spawn(move || {
                runtime.block_on(start_gui_client(
                    addr, values, rx, sender, client, version, token,
                ))
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                start_gui_client(addr, values, rx, sender, client, version, token).await;
            });
        }

        (states, client_out)
    }
}
