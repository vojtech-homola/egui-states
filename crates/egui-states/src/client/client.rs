//! Client construction, connection control, and egui repaint integration.

use std::any::Any;
use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::panic::AssertUnwindSafe;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use egui::Context;
use futures_util::FutureExt;
use futures_util::future::{AbortHandle, Abortable, Either, select};
use parking_lot::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::PROTOCOL_VERSION;
use crate::State;
use crate::client::messages::{ChannelMessage, MessageSender, MessagesSerializer, handle_message};
use crate::client::states_creator::{StatesCreatorClient, ValuesList};
use crate::event::Event;
use crate::protocol::ClientHeader;

#[cfg(not(target_arch = "wasm32"))]
use crate::client::net::websocket::{WsClientRead, WsClientSend, build_ws};

#[cfg(target_arch = "wasm32")]
use crate::client::net::websocket_wasm::{WsClientRead, WsClientSend, build_ws};

type MessageReceiver = UnboundedReceiver<Option<ChannelMessage>>;

async fn start_gui_client(
    url: String,
    vals: ValuesList,
    mut rx: MessageReceiver,
    sender: MessageSender,
    client: ClientCore,
    version: Option<u64>,
    hash: Option<String>,
) {
    // Every exit from this loop is a shutdown: the user dropped the last
    // `Client` handle, so nothing can observe the connection state anymore.
    loop {
        // wait for the connection signal
        if client.is_shut_down()
            || client
                .until_shutdown(client.wait_connection())
                .await
                .is_none()
        {
            return;
        }
        client.set_state(ConnectionState::NotConnected);

        // try to connect to the server
        let Some(socket) = client.until_shutdown(build_ws(&url)).await else {
            return;
        };
        let (socket_read, mut socket_send) = match socket {
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
        // Nothing is queued for this connection yet, so a shutdown can drop it.
        let Some(sent) = client.until_shutdown(socket_send.send(message)).await else {
            return;
        };
        if sent.is_err() {
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
    client: &ClientCore,
) -> Result<(MessageReceiver, SessionEnd), String> {
    // A panicking send task stops the receive task; reading alone would keep
    // the connection open while nothing can be sent.
    let (abort_receive, abort_registration) = AbortHandle::new_pair();

    let recv_future = {
        let (vals, client, sender) = (vals.clone(), client.clone(), sender.clone());
        async move {
            // A shutdown stops reading at once; the send task still writes the
            // messages queued so far and closes the socket.
            let read =
                client.until_shutdown(catch_panic(receive(socket_read, &vals, &client, &sender)));
            let end = Abortable::new(read, abort_registration).await;
            // Closing the queue ends the send task.
            sender.close();
            // The read is aborted only after the send task panicked, which is
            // reported instead.
            end.ok().flatten().unwrap_or(Ok(SessionEnd::Closed))
        }
    };

    let send_future = {
        let client = client.clone();
        async move {
            // The serializer owns the queue receiver and stays outside the panic
            // guard, so the receiver survives a panic.
            let mut serializer = MessagesSerializer::new(rx);
            // After a shutdown the queued messages get a bounded time to go
            // out; a server that stops reading cannot keep the task alive.
            let write = catch_panic(send(&mut serializer, socket_send));
            let end = client.until_flush_deadline(write).await.unwrap_or(Ok(()));
            if end.is_err() {
                abort_receive.abort();
            }
            (serializer.close(), end)
        }
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
    client: &ClientCore,
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
    context: Context,
    connect_signal: Event,
    // Set once, when the last `Client` handle is dropped.
    shutdown: Event,
    // The error is set only together with `ConnectionState::Failed`.
    state: RwLock<(ConnectionState, Option<String>)>,
    sender: MessageSender,
}

/// How long a shut-down client keeps writing queued messages before it drops
/// the connection.
#[cfg(not(target_arch = "wasm32"))]
const SHUTDOWN_FLUSH_GRACE: Duration = Duration::from_secs(1);

/// Runs `future` until it completes or `stop` does; `None` if `stop` won.
async fn until<F: Future>(future: F, stop: impl Future) -> Option<F::Output> {
    match select(pin!(future), pin!(stop)).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(_) => None,
    }
}

/// Shared client state used by the background connection task.
///
/// Unlike [`Client`], holding a `ClientCore` does not keep the background task
/// running, so the task can hold one without keeping itself alive.
#[derive(Clone)]
pub(crate) struct ClientCore(Arc<ClientInner>);

impl ClientCore {
    pub(crate) fn new(context: Context, sender: MessageSender) -> Self {
        let inner = ClientInner {
            context,
            connect_signal: Event::new(),
            shutdown: Event::new(),
            state: RwLock::new((ConnectionState::NotConnected, None)),
            sender,
        };

        Self(Arc::new(inner))
    }

    /// Requests an egui repaint immediately, or after `time` seconds if positive.
    pub(crate) fn update(&self, time: f32) {
        let ctx = &self.0.context;
        // The delay comes from the server and must not panic this task.
        match Duration::try_from_secs_f32(time) {
            Ok(delay) if !delay.is_zero() => ctx.request_repaint_after(delay),
            _ => ctx.request_repaint(),
        }
    }

    pub(crate) async fn wait_connection(&self) {
        self.0.connect_signal.wait_clear_async().await;
    }

    fn is_shut_down(&self) -> bool {
        self.0.shutdown.is_set()
    }

    /// Runs `future` until it completes or the client shuts down.
    ///
    /// Returns `None` if the client shut down first.
    async fn until_shutdown<F: Future>(&self, future: F) -> Option<F::Output> {
        until(future, self.0.shutdown.wait_async()).await
    }

    /// Runs `future` until it completes or [`SHUTDOWN_FLUSH_GRACE`] has passed
    /// since the client shut down.
    ///
    /// Returns `None` if the grace period ran out first. On WASM the browser
    /// buffers socket writes, so `future` always runs to completion.
    async fn until_flush_deadline<F: Future>(&self, future: F) -> Option<F::Output> {
        let deadline = async {
            self.0.shutdown.wait_async().await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(SHUTDOWN_FLUSH_GRACE).await;
            #[cfg(target_arch = "wasm32")]
            std::future::pending::<()>().await;
        };
        until(future, deadline).await
    }

    pub(crate) fn set_state(&self, state: ConnectionState) {
        *self.0.state.write() = (state, None);
        self.0.context.request_repaint();
    }

    pub(crate) fn fail(&self, error: String) {
        print_error(&error);
        *self.0.state.write() = (ConnectionState::Failed, Some(error));
        self.0.context.request_repaint();
    }

    #[inline]
    pub(crate) fn send_ack(&self, id: u64) {
        self.0.sender.send_ack(id);
    }
}

/// Owns the background task; dropping it shuts the task down.
struct ClientOwner(ClientCore);

impl Drop for ClientOwner {
    fn drop(&mut self) {
        self.0.0.shutdown.set();
    }
}

/// Handle for controlling the background state-synchronization client.
///
/// Clones share one background task. Dropping the last clone closes the
/// active connection, if any, and stops the task; on native targets its
/// threads exit. Messages queued before the drop are still sent, but on
/// native targets the connection is dropped after one second even if some
/// could not be written. The state tree returned by [`ClientBuilder::build`] stays
/// usable but keeps its last values and no longer synchronizes.
#[derive(Clone)]
pub struct Client(Arc<ClientOwner>);

impl Client {
    fn new(core: ClientCore) -> Self {
        Self(Arc::new(ClientOwner(core)))
    }

    fn core(&self) -> &ClientCore {
        &self.0.0
    }

    /// Requests an egui repaint immediately, or after `time` seconds if positive.
    ///
    /// A delay that is not finite or does not fit in a [`Duration`] requests an
    /// immediate repaint.
    pub fn update(&self, time: f32) {
        self.core().update(time);
    }

    /// Starts or retries a connection to the configured server asynchronously.
    ///
    /// A failed attempt waits for another call to `connect`; it does not block
    /// the calling UI thread.
    pub fn connect(&self) {
        self.core().0.connect_signal.set();
    }

    /// Closes the active connection.
    pub fn disconnect(&self) {
        self.core().0.sender.close();
    }

    /// Returns the current connection status.
    pub fn get_state(&self) -> ConnectionState {
        self.core().0.state.read().0
    }

    /// Returns why the client is in [`ConnectionState::Failed`].
    ///
    /// Returns `None` in every other state; a new connection attempt clears
    /// the error.
    pub fn last_error(&self) -> Option<String> {
        self.core().0.state.read().1.clone()
    }

    /// Makes the send task panic on the next queued message.
    #[cfg(test)]
    pub(crate) fn inject_send_panic(&self) {
        self.core().0.sender.send(ChannelMessage::TestPanic);
    }
}

/// Configures and builds a typed client and its root state tree.
pub struct ClientBuilder<T> {
    creator: StatesCreatorClient,
    states: T,
    sender: MessageSender,
    rx: UnboundedReceiver<Option<ChannelMessage>>,
    host: Option<String>,
    port: u16,
    context: Context,
    version: Option<u64>,
    token: Option<String>,
}

impl<T> ClientBuilder<T>
where
    T: State,
{
    /// Creates the state tree and a builder targeting the server on `port`.
    ///
    /// Without [`Self::host`] or [`Self::addr`], a native client connects to
    /// `127.0.0.1:port`. A WASM client connects back to the host and port its
    /// page was loaded from, so a page served by the state server reaches it
    /// through any address, tunnel, or forwarded port; `port` is then used
    /// only when the page has no host, such as a page opened from a file.
    /// WASM clients use `wss` when the page was loaded over HTTPS.
    ///
    /// The client requests repaints of `context` when the connection state
    /// changes or the server asks for an update. No application version or
    /// authentication token is set by default. The derived [`State::new`]
    /// implementation runs during this call.
    pub fn new(context: Context, port: u16) -> Self {
        let (sender, rx) = MessageSender::new();

        let mut creator =
            StatesCreatorClient::new(sender.clone(), context.clone(), "root".to_string());
        let states = T::new(&mut creator);

        Self {
            creator,
            states,
            sender,
            rx,
            host: None,
            port,
            context,
            version: None,
            token: None,
        }
    }

    /// Sets the server IPv4 address.
    ///
    /// Equivalent to [`Self::host`] with the address.
    pub fn addr(self, addr: Ipv4Addr) -> Self {
        self.host(addr.to_string())
    }

    /// Sets the server host: a host name such as `myserver.local`, or an IPv4
    /// or IPv6 address, without scheme or port.
    ///
    /// The client connects to this host on the builder's port, also on WASM,
    /// where it replaces the default of the page's own host and port.
    pub fn host(self, host: impl Into<String>) -> Self {
        Self {
            host: Some(host.into()),
            ..self
        }
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

    /// Returns the stable hash of the configured state tree.
    ///
    /// Generated server bindings expose the same hash so mismatched state
    /// layouts can be rejected during the handshake.
    pub fn get_version_hash(&self) -> u64 {
        self.creator.get_version_hash()
    }

    /// Builds the root state and starts the background client task.
    ///
    /// The task waits until [`Client::connect`] is called before opening a
    /// connection. On native targets it owns a dedicated Tokio runtime; on WASM
    /// it is spawned on the current browser executor. Keep the returned
    /// [`Client`] to connect, disconnect, inspect status, and request repaints;
    /// dropping its last clone stops the task and ends synchronization.
    ///
    /// For native debugging, the owner thread is named `Client`; its two Tokio
    /// workers use the `Client Runtime` prefix.
    pub fn build(self) -> (T, Client) {
        let Self {
            creator,
            states,
            sender,
            rx,
            host,
            port,
            context,
            version,
            token,
        } = self;

        let url = server_url(host.as_deref(), port);
        let values = creator.get_values();
        let client = ClientCore::new(context, sender.clone());
        let client_out = Client::new(client.clone());

        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::thread;
            use tokio::runtime::Builder;

            let runtime = Builder::new_multi_thread()
                .thread_name("Client Runtime")
                .enable_io()
                .enable_time()
                .worker_threads(2)
                .build()
                .unwrap();

            let thread = thread::Builder::new().name("Client".to_string());

            let _ = thread.spawn(move || {
                runtime.block_on(start_gui_client(
                    url, values, rx, sender, client, version, token,
                ))
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                start_gui_client(url, values, rx, sender, client, version, token).await;
            });
        }

        (states, client_out)
    }
}

/// Formats `host` for a URL, bracketing an IPv6 address.
fn url_host(host: &str) -> String {
    match host.parse::<Ipv6Addr>() {
        Ok(_) => format!("[{host}]"),
        Err(_) => host.to_string(),
    }
}

/// Returns the WebSocket URL of the server.
#[cfg(not(target_arch = "wasm32"))]
fn server_url(host: Option<&str>, port: u16) -> String {
    format!("ws://{}:{port}/ws", url_host(host.unwrap_or("127.0.0.1")))
}

/// Returns the WebSocket URL of the server.
///
/// Without an explicit host this is the page's own host and port, read once
/// when the client is built.
#[cfg(target_arch = "wasm32")]
fn server_url(host: Option<&str>, port: u16) -> String {
    let location = web_sys::window().map(|window| window.location());
    let secure = location
        .as_ref()
        .and_then(|location| location.protocol().ok())
        .is_some_and(|protocol| protocol == "https:");
    let scheme = if secure { "wss" } else { "ws" };
    let page_host = || {
        location
            .as_ref()
            .and_then(|location| location.host().ok())
            .filter(|host| !host.is_empty())
    };
    match host {
        Some(host) => format!("{scheme}://{}:{port}/ws", url_host(host)),
        None => match page_host() {
            Some(page_host) => format!("{scheme}://{page_host}/ws"),
            None => format!("{scheme}://127.0.0.1:{port}/ws"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_default_to_loopback_and_accept_host_names() {
        assert_eq!(server_url(None, 8091), "ws://127.0.0.1:8091/ws");
        assert_eq!(
            server_url(Some("myserver.local"), 8091),
            "ws://myserver.local:8091/ws"
        );
        assert_eq!(server_url(Some("10.0.0.2"), 80), "ws://10.0.0.2:80/ws");
        assert_eq!(server_url(Some("::1"), 8091), "ws://[::1]:8091/ws");
    }

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
        let client = ClientCore::new(context, sender);
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

    /// A server that stops reading cannot keep a dropped client alive: the
    /// queued messages get a bounded time to go out, then the connection is
    /// dropped.
    #[cfg(target_os = "linux")]
    #[test]
    fn dropped_client_stops_when_the_server_stops_reading() {
        use crate::test_support::client_thread_count;
        use futures_util::StreamExt;

        struct LargeValueState {
            value: crate::Value<String>,
        }

        impl crate::State for LargeValueState {
            const NAME: &'static str = "LargeValueState";

            fn new(c: &mut impl crate::StatesCreator) -> Self {
                Self {
                    value: c.value("value", String::new()),
                }
            }
        }

        if crate::test_support::isolated() {
            return;
        }

        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let peer = std::thread::Builder::new()
            .name("peer".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .build()
                    .unwrap();
                runtime.block_on(async move {
                    listener.set_nonblocking(true).unwrap();
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    // The protocol handshake follows the client's queue cleanup,
                    // so values set from now on are sent.
                    ws.next().await.unwrap().unwrap();
                    ready_tx.send(()).unwrap();
                    // Never read again, so the client's socket buffers fill up.
                    let _ = stop_rx.await;
                });
            })
            .unwrap();

        let (state, client) =
            ClientBuilder::<LargeValueState>::new(egui::Context::default(), port).build();
        client.connect();
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // 50 MiB is far more than the socket buffers hold.
        let text = "x".repeat(512 * 1024);
        for _ in 0..100 {
            state.value.set(text.clone());
        }
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(client_thread_count(), 3);

        drop(client);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while client_thread_count() != 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out: client shutdown"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        stop_tx.send(()).unwrap();
        peer.join().unwrap();
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
