//! Client construction, connection control, and egui repaint integration.

use std::any::Any;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use egui::Context;
use futures_util::FutureExt;
use futures_util::future::{AbortHandle, Abortable};
use parking_lot::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::PROTOCOL_VERSION;
use crate::State;
use crate::client::messages::{ChannelMessage, MessageSender, MessagesSerializer, handle_message};
use crate::client::states_creator::{StatesCreatorClient, ValuesList};
use crate::event::Event;
use crate::serialization::ClientHeader;

#[cfg(not(target_arch = "wasm32"))]
use crate::client::websocket::{WsClientRead, WsClientSend, build_ws};

#[cfg(target_arch = "wasm32")]
use crate::client::websocket_wasm::{WsClientRead, WsClientSend, build_ws};

type MessageReceiver = UnboundedReceiver<Option<ChannelMessage>>;

async fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    mut rx: MessageReceiver,
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
        let (socket_read, mut socket_send) = match build_ws(addr).await {
            Ok(socket) => socket,
            Err(error) => {
                client.fail(error);
                continue;
            }
        };

        // clean message queue before starting
        while !rx.is_empty() {
            let _ = rx.recv().await;
        }

        // communicate handshake and initialization -------------------------
        let message = ClientHeader::serialize_handshake(PROTOCOL_VERSION, version, hash.clone());
        if socket_send.send(message).await.is_err() {
            client.fail("Sending handshake failed.".to_string());
            continue;
        }

        let session = run_session(socket_read, socket_send, rx, &vals, &sender, &client).await;
        let end = match session {
            Ok((rx_back, end)) => {
                rx = rx_back;
                end
            }
            Err(error) => {
                // Without the message queue no further connection can work.
                vals.disconnected();
                client.fail(format!(
                    "Client connection task stopped, build a new client: {}",
                    error
                ));
                return;
            }
        };

        vals.disconnected();
        // Requests made while connected do not start a reconnect.
        client.0.connect_signal.clear();
        match end {
            SessionEnd::Closed => client.set_state(ConnectionState::Disconnected),
            SessionEnd::NotAccepted(error) => client.fail(format!(
                "Connection closed before the server accepted the handshake: {}",
                error
            )),
            SessionEnd::Panicked(task, message) => {
                client.fail(format!("Client {} task panicked: {}", task, message))
            }
        }
    }
}

/// How one connection ended.
enum SessionEnd {
    /// An established connection ended.
    Closed,
    /// The connection ended before the server accepted the handshake.
    NotAccepted(String),
    /// A connection task panicked; holds the task name and the panic message.
    Panicked(&'static str, String),
}

/// Runs the receive and send tasks of one connection until both end.
///
/// Returns the message queue receiver for the next connection and how this
/// one ended. A panic in either task ends the connection instead of the
/// client. The error case means the receiver was lost, which only happens if
/// the send task stopped outside its panic guard.
async fn run_session(
    socket_read: WsClientRead,
    socket_send: WsClientSend,
    rx: MessageReceiver,
    vals: &ValuesList,
    sender: &MessageSender,
    client: &Client,
) -> Result<(MessageReceiver, SessionEnd), String> {
    // A panicking send task stops the receive task; reading alone would keep
    // the connection open while nothing can be sent.
    let (abort_receive, abort_registration) = AbortHandle::new_pair();

    let recv_future = {
        let (vals, client, sender) = (vals.clone(), client.clone(), sender.clone());
        async move {
            let read = catch_panic(receive(socket_read, &vals, &client, &sender));
            let end = Abortable::new(read, abort_registration).await;
            // Closing the queue ends the send task.
            sender.close();
            // The read is aborted only after the send task panicked, which is
            // reported instead.
            end.unwrap_or(Ok(SessionEnd::Closed))
        }
    };

    let send_future = async move {
        // The serializer owns the queue receiver and stays outside the panic
        // guard, so the receiver survives a panic.
        let mut serializer = MessagesSerializer::new(rx);
        let end = catch_panic(send(&mut serializer, socket_send)).await;
        if end.is_err() {
            abort_receive.abort();
        }
        (serializer.close(), end)
    };

    #[cfg(not(target_arch = "wasm32"))]
    let (recv_end, send_end) = {
        let recv_task = tokio::spawn(recv_future);
        let send_task = tokio::spawn(send_future);
        let recv_end = recv_task.await.unwrap_or_else(|error| {
            // The task stopped before closing the queue.
            sender.close();
            Err(join_error_message(error))
        });
        let send_end = send_task.await.map_err(join_error_message)?;
        (recv_end, send_end)
    };

    #[cfg(target_arch = "wasm32")]
    let (recv_end, send_end) = tokio::join!(recv_future, send_future);

    let (rx, send_end) = send_end;
    let end = match (recv_end, send_end) {
        (Err(message), _) => SessionEnd::Panicked("receive", message),
        (Ok(_), Err(message)) => SessionEnd::Panicked("send", message),
        (Ok(end), Ok(())) => end,
    };
    Ok((rx, end))
}

/// Reads and applies server messages until the connection ends.
async fn receive(
    mut socket_read: WsClientRead,
    vals: &ValuesList,
    client: &Client,
    sender: &MessageSender,
) -> SessionEnd {
    // The server answers an accepted handshake with its synchronization
    // messages and sends nothing to a rejected client.
    let mut accepted = false;
    let error = loop {
        match socket_read.read().await {
            Ok(msg) => {
                if !accepted {
                    accepted = true;
                    client.set_state(ConnectionState::Connected);
                }
                if let Err(e) = handle_message(msg, vals, client).await {
                    let error = format!("handling message from server failed: {:?}", e);
                    sender.send_message(&error);
                    print_error(&error);
                    // break; TODO: decide if we want to break the loop on error
                }
            }
            Err(e) => break e,
        }
    };

    if accepted {
        print_error(&format!("Connection with server failed: {:?}", error));
        SessionEnd::Closed
    } else {
        SessionEnd::NotAccepted(error.to_string())
    }
}

/// Writes queued messages until the queue closes or the socket fails.
async fn send(serializer: &mut MessagesSerializer, mut socket_send: WsClientSend) {
    while let Some(message) = serializer.next().await {
        if socket_send.send(message).await.is_err() {
            break;
        }
    }
    socket_send.close().await;
}

/// Runs `future`, turning a panic into its message.
///
/// After a panic the connection is torn down: `ValuesList::disconnected`
/// resets partial transfers and the next connection resynchronizes every
/// value, so no state observed during the panic outlives it.
async fn catch_panic<F: Future>(future: F) -> Result<F::Output, String> {
    AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .map_err(panic_message)
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => message.to_string(),
            Err(_) => "unknown panic payload".to_string(),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn join_error_message(error: tokio::task::JoinError) -> String {
    match error.try_into_panic() {
        Ok(payload) => panic_message(payload),
        Err(error) => error.to_string(),
    }
}

pub(crate) fn print_error(error: &str) {
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    println!("{}", error);
    #[cfg(all(debug_assertions, target_arch = "wasm32"))]
    log::error!("{}", error);
    let _ = error;
}

/// Current client connection status.
///
/// A connection attempt starts in [`Self::NotConnected`], becomes
/// [`Self::Connected`] once the server accepts the handshake, and becomes
/// [`Self::Disconnected`] when an established connection ends. An attempt that
/// does not reach an established connection, or a connection whose background
/// task panicked, ends in [`Self::Failed`]. Calling [`Client::connect`] starts
/// a new attempt and returns the state to `NotConnected`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionState {
    /// No connection has been established or a connection attempt is pending.
    NotConnected,
    /// The server accepted the handshake and state synchronization is active.
    Connected,
    /// A previously active connection ended.
    Disconnected,
    /// The server could not be reached, did not accept the handshake, or a
    /// background connection task panicked. [`Client::last_error`] describes
    /// the cause; call [`Client::connect`] to retry.
    Failed,
}

struct ClientInner {
    context: Option<Context>,
    connect_signal: Event,
    // The error is set only together with `ConnectionState::Failed`.
    state: RwLock<(ConnectionState, Option<String>)>,
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
            state: RwLock::new((ConnectionState::NotConnected, None)),
            sender,
        };

        Self(Arc::new(inner))
    }

    /// Sets the egui context used to request repaints after incoming updates.
    ///
    /// This must be called before cloning the client; use
    /// [`ClientBuilder::context`] when possible.
    ///
    /// # Panics
    ///
    /// Panics if this [`Client`] has already been cloned.
    pub fn set_context(&mut self, context: Context) {
        Arc::get_mut(&mut self.0).unwrap().set_context(context);
    }

    /// Requests an egui repaint immediately, or after `time` seconds if positive.
    ///
    /// A delay that is not finite or does not fit in a [`Duration`] requests an
    /// immediate repaint.
    pub fn update(&self, time: f32) {
        if let Some(ctx) = &self.0.context {
            // The delay comes from the server and must not panic this task.
            match Duration::try_from_secs_f32(time) {
                Ok(delay) if !delay.is_zero() => ctx.request_repaint_after(delay),
                _ => ctx.request_repaint(),
            }
        }
    }

    pub(crate) async fn wait_connection(&self) {
        self.0.connect_signal.wait_clear_async().await;
    }

    /// Starts or retries a connection to the configured server asynchronously.
    ///
    /// A failed attempt waits for another call to `connect`; it does not block
    /// the calling UI thread.
    pub fn connect(&self) {
        self.0.connect_signal.set();
    }

    /// Closes the active connection.
    pub fn disconnect(&self) {
        self.0.sender.close();
    }

    pub(crate) fn set_state(&self, state: ConnectionState) {
        *self.0.state.write() = (state, None);
        if let Some(ctx) = &self.0.context {
            ctx.request_repaint();
        }
    }

    pub(crate) fn fail(&self, error: String) {
        print_error(&error);
        *self.0.state.write() = (ConnectionState::Failed, Some(error));
        if let Some(ctx) = &self.0.context {
            ctx.request_repaint();
        }
    }

    #[inline]
    pub(crate) fn send_ack(&self, id: u64) {
        self.0.sender.send_ack(id);
    }

    /// Returns the current connection status.
    pub fn get_state(&self) -> ConnectionState {
        self.0.state.read().0
    }

    /// Returns why the client is in [`ConnectionState::Failed`].
    ///
    /// Returns `None` in every other state; a new connection attempt clears
    /// the error.
    pub fn last_error(&self) -> Option<String> {
        self.0.state.read().1.clone()
    }

    /// Makes the send task panic on the next queued message.
    #[cfg(test)]
    pub(crate) fn inject_send_panic(&self) {
        self.0.sender.send(ChannelMessage::TestPanic);
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
    /// Creates the state tree and a builder targeting `127.0.0.1`.
    ///
    /// No egui context, application version, or authentication token is set by
    /// default. The derived [`State::new`] implementation runs during this call.
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
    /// it is spawned on the current browser executor. Keep the returned
    /// [`Client`] to connect, disconnect, inspect status, and request repaints.
    ///
    /// For native debugging, the owner thread is named `Client`; its two Tokio
    /// workers use the `Client Runtime` prefix.
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

impl<T> Default for ClientBuilder<T>
where
    T: State,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns the repaint requests made by `Client::update(delay)`.
    fn repaint_requests(delay: f32) -> Vec<Duration> {
        let context = egui::Context::default();
        // Drain initial repaint requests so the requested one is observable.
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    predicted_dt: 0.0,
                    ..Default::default()
                },
                |_| {},
            );
            output.textures_delta.clear();
        }
        let requests = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let received = requests.clone();
        context.set_request_repaint_callback(move |info| received.lock().push(info.delay));
        let (sender, _receiver) = MessageSender::new();
        let client = Client::new(Some(context), sender);
        client.update(delay);
        requests.lock().clone()
    }

    #[test]
    fn update_requests_immediate_and_delayed_repaints() {
        for delay in [0.0, 0.25] {
            assert_eq!(
                repaint_requests(delay),
                [Duration::from_secs_f32(delay)],
                "delay {delay}"
            );
        }
    }

    /// Delays that are not finite or too large for a `Duration` request an
    /// immediate repaint instead of panicking.
    #[test]
    fn update_with_an_unrepresentable_delay_repaints_immediately() {
        for delay in [f32::INFINITY, f32::NAN, 1e20, -1.0] {
            assert_eq!(repaint_requests(delay), [Duration::ZERO], "delay {delay}");
        }
    }

    #[test]
    fn catch_panic_returns_the_panic_message() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert_eq!(catch_panic(async { 3 }).await, Ok(3));
            let literal = catch_panic(async { panic!("literal") }).await;
            assert_eq!(literal, Err::<(), _>("literal".to_string()));
            let formatted = catch_panic(async { panic!("formatted {}", 7) }).await;
            assert_eq!(formatted, Err::<(), _>("formatted 7".to_string()));
            let other = catch_panic(async { std::panic::panic_any(7_u8) }).await;
            assert_eq!(other, Err::<(), _>("unknown panic payload".to_string()));
        });
    }
}
