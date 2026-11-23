//! Client construction, connection control, and egui repaint integration.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::net::Ipv6Addr;
use std::panic::AssertUnwindSafe;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use egui::Context;
use futures_util::FutureExt;
use futures_util::future::{AbortHandle, Abortable, Either, select};
use parking_lot::RwLock;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::PROTOCOL_VERSION;
use crate::State;
use crate::client::messages::{
    ChannelMessage, MessageSender, MessagesSerializer, ReadError, ServerFrames, ServerMessage,
    ServerReader, handle_message,
};
use crate::client::net::timer::{self, Sleep};
use crate::client::states_creator::{StatesCreatorClient, ValuesList};
use crate::event::Event;
use crate::protocol::close;
use crate::protocol::{ClientHeader, FastVec, ServerHeader};

use crate::client::net::Target;
#[cfg(not(target_arch = "wasm32"))]
use crate::client::net::websocket::{WsClientRead, WsClientSend, build_ws};
#[cfg(feature = "tls")]
use crate::client::tls::Tls;

#[cfg(target_arch = "wasm32")]
use crate::client::net::websocket_wasm::{WsClientRead, WsClientSend, build_ws};

type MessageReceiver = UnboundedReceiver<Option<ChannelMessage>>;

/// How long a connection attempt waits by default for the server to answer.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// What the client waits beyond the time the server gave for its connect
/// request, so a late answer is not cut off by network delay.
const APPROVAL_MARGIN: Duration = Duration::from_secs(2);

/// What the connection task needs to open a connection.
struct Settings {
    target: Target,
    version: Option<u64>,
    token: Option<String>,
    connect_timeout: Duration,
    reconnect: Option<Reconnect>,
}

/// When the client connects again by itself after a connection attempt
/// failed or a connection ended.
///
/// Set it with [`ClientBuilder::reconnect`]. The client retries only when
/// the cause can pass:
///
/// - [`CloseReason::Unreachable`], [`CloseReason::Timeout`],
///   [`CloseReason::ConnectionLost`] and [`CloseReason::ServerStopped`] after
///   [`Self::delay`], which doubles with each failed attempt up to
///   [`Self::max_delay`];
/// - [`CloseReason::Busy`] after [`Self::busy_delay`], since another client
///   holds the connection. The server drops a client that stopped answering
///   after its heartbeat timeout, so a connection the network left half-open
///   frees the server in time.
///
/// Every other reason waits for [`Client::connect`], as without reconnecting:
/// a version, token or protocol mismatch would fail again, a refusal was the
/// server's decision, and retrying after [`CloseReason::Replaced`] would take
/// the connection back from the client that replaced this one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reconnect {
    /// How long the client waits before the first new attempt. Default
    /// 500 ms.
    pub delay: Duration,
    /// The longest wait between attempts. Default 5 s.
    pub max_delay: Duration,
    /// How long the client waits after the server refused it as
    /// [`CloseReason::Busy`]. Default 5 s.
    pub busy_delay: Duration,
}

impl Default for Reconnect {
    fn default() -> Self {
        Self {
            delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(5),
            busy_delay: Duration::from_secs(5),
        }
    }
}

/// The waits of the client's automatic reconnecting.
struct Retry {
    policy: Option<Reconnect>,
    /// The wait before the next attempt after an unreachable server or a
    /// lost connection.
    delay: Duration,
}

impl Retry {
    fn new(policy: Option<Reconnect>) -> Self {
        let delay = policy.map_or(Duration::ZERO, |policy| policy.delay);
        Self { policy, delay }
    }

    /// Restarts the waits from the policy's first delay.
    fn reset(&mut self) {
        *self = Self::new(self.policy);
    }

    /// How long to wait before the next attempt after one that ended for
    /// `reason`; `None` when the client waits for `Client::connect`.
    fn after(&mut self, reason: Option<CloseReason>) -> Option<Duration> {
        let policy = self.policy?;
        match reason? {
            CloseReason::Busy => Some(policy.busy_delay),
            CloseReason::Unreachable
            | CloseReason::Timeout
            | CloseReason::ConnectionLost
            | CloseReason::ServerStopped => {
                let delay = self.delay.min(policy.max_delay);
                self.delay = delay.saturating_mul(2).min(policy.max_delay);
                Some(delay)
            }
            _ => None,
        }
    }

    /// Ends an attempt that failed for `reason`, in `Reconnecting` or
    /// `Failed`; returns the wait before the next attempt.
    fn failed(
        &mut self,
        client: &ClientCore,
        reason: Option<CloseReason>,
        error: String,
    ) -> Option<Duration> {
        let next = self.after(reason);
        match next {
            Some(_) => client.reconnecting(reason, Some(error)),
            None => client.fail(reason, error),
        }
        next
    }

    /// Ends an established connection that ended for `reason`, in
    /// `Reconnecting` or `Disconnected`; returns the wait before the next
    /// attempt.
    fn disconnected(
        &mut self,
        client: &ClientCore,
        reason: Option<CloseReason>,
        error: Option<String>,
    ) -> Option<Duration> {
        let next = self.after(reason);
        match next {
            Some(_) => client.reconnecting(reason, error),
            None => client.disconnected(reason, error),
        }
        next
    }
}

/// How the wait for the next connection attempt ended.
enum Wake {
    /// The attempt is due.
    Attempt,
    /// The user disconnected, which stops the reconnecting.
    Cancelled,
}

/// Waits `delay` for an automatic attempt, or less if the user calls
/// `Client::connect`.
async fn wait_retry(delay: Duration, rx: &mut MessageReceiver, client: &ClientCore) -> Wake {
    let requested = pin!(client.wait_connection());
    let due = select(timer::sleep(delay), requested);
    match select(due, pin!(disconnect_requested(rx, client))).await {
        Either::Left(_) => Wake::Attempt,
        Either::Right(_) => Wake::Cancelled,
    }
}

/// Completes once the user calls `Client::disconnect`.
///
/// It reads the message queue, dropping what the UI queued: outside a
/// connection it belongs to none. A disconnect made before the last call to
/// `Client::connect` is ignored; it left only its close in the queue.
async fn disconnect_requested(rx: &mut MessageReceiver, client: &ClientCore) {
    loop {
        match rx.recv().await {
            Some(None) if client.disconnect_requested() => return,
            Some(_) => {}
            // The connection task holds a sender, so the queue never closes.
            None => std::future::pending().await,
        }
    }
}

/// Drops what the UI queued, and tells whether the user called
/// `Client::disconnect` meanwhile, as [`disconnect_requested`] does.
fn drain_queue(rx: &mut MessageReceiver, client: &ClientCore) -> bool {
    let mut disconnected = false;
    while let Ok(message) = rx.try_recv() {
        disconnected |= message.is_none() && client.disconnect_requested();
    }
    disconnected
}

async fn start_gui_client(
    settings: Settings,
    vals: ValuesList,
    mut rx: MessageReceiver,
    sender: MessageSender,
    client: ClientCore,
) {
    let mut retry = Retry::new(settings.reconnect);
    // The wait before the next automatic attempt; `None` waits for
    // `Client::connect`.
    let mut next: Option<Duration> = None;
    // Every exit from this loop is a shutdown: the user dropped the last
    // `Client` handle, so nothing can observe the connection state anymore.
    loop {
        if client.is_shut_down() {
            return;
        }
        let wake = match next.take() {
            Some(delay) => {
                client
                    .until_shutdown(wait_retry(delay, &mut rx, &client))
                    .await
            }
            None => {
                retry.reset();
                let requested = client.until_shutdown(client.wait_connection()).await;
                requested.map(|()| Wake::Attempt)
            }
        };
        let Some(wake) = wake else {
            return;
        };
        if let Wake::Cancelled = wake {
            retry.reset();
            client.set_state(ConnectionState::NotConnected);
            continue;
        }
        client.set_state(ConnectionState::Connecting);

        // On native targets the timeout bounds the whole attempt, and a
        // disconnect cancels the opening. In the browser the timeout starts
        // once the socket is open: the browser bounds the opening itself, and
        // an abandoned opening would leave its socket, so a disconnect waits
        // for the opening to end.
        #[cfg(not(target_arch = "wasm32"))]
        let mut deadline = timer::sleep(settings.connect_timeout);
        #[cfg(not(target_arch = "wasm32"))]
        let opening = until(
            until(build_ws(&settings.target), deadline.as_mut()),
            disconnect_requested(&mut rx, &client),
        );
        #[cfg(target_arch = "wasm32")]
        let opening = async { Some(Some(build_ws(&settings.target).await)) };

        // try to connect to the server
        let Some(opened) = client.until_shutdown(opening).await else {
            return;
        };
        let (socket_read, mut socket_send) = match opened {
            Some(Some(Ok(socket))) => socket,
            Some(Some(Err(error))) => {
                next = retry.failed(&client, Some(CloseReason::Unreachable), error);
                continue;
            }
            Some(None) => {
                let error = no_answer(settings.connect_timeout);
                next = retry.failed(&client, Some(CloseReason::Timeout), error);
                continue;
            }
            None => {
                client.set_state(ConnectionState::NotConnected);
                continue;
            }
        };
        #[cfg(target_arch = "wasm32")]
        let deadline = timer::sleep(settings.connect_timeout);

        // What the UI queued so far belongs to no connection.
        if drain_queue(&mut rx, &client) {
            let _ = client.until_shutdown(socket_send.close()).await;
            client.set_state(ConnectionState::NotConnected);
            continue;
        }

        // communicate handshake and initialization -------------------------
        let message = ClientHeader::serialize_handshake(
            PROTOCOL_VERSION,
            settings.version,
            settings.token.clone(),
        );
        // Nothing is queued for this connection yet, so a shutdown can drop it.
        let Some(sent) = client.until_shutdown(socket_send.send(message)).await else {
            return;
        };
        if let Err(error) = sent {
            let error = format!("Sending handshake failed: {error}");
            next = retry.failed(&client, Some(CloseReason::ConnectionLost), error);
            continue;
        }

        let attempt = Attempt {
            deadline,
            timeout: settings.connect_timeout,
        };
        let mut socket_read = socket_read;
        let answer = catch_panic(await_answer(&mut socket_read, &mut rx, &client, attempt));
        let Some(answer) = client.until_shutdown(answer).await else {
            return;
        };
        let first = match answer {
            Ok(Answer::Accepted(first)) => first,
            Ok(Answer::NotAccepted(reason, error)) => {
                client.end_attempt();
                next = retry.failed(&client, Some(reason), error);
                continue;
            }
            Ok(Answer::Cancelled) => {
                let _ = client.until_shutdown(socket_send.close()).await;
                client.end_attempt();
                client.set_state(ConnectionState::NotConnected);
                continue;
            }
            Err(message) => {
                client.end_attempt();
                client.fail(None, format!("Client receive task panicked: {message}"));
                continue;
            }
        };
        // Take values from the closed connection were takeable until now.
        vals.connected();
        client.set_state(ConnectionState::Connected);
        retry.reset();

        let session =
            run_session(socket_read, socket_send, rx, &vals, &sender, &client, first).await;
        let end = match session {
            Ok((rx_back, end)) => {
                rx = rx_back;
                end
            }
            Err(error) => {
                // Without the message queue no further connection can work.
                vals.disconnected();
                client.fail(
                    None,
                    format!(
                        "Client connection task stopped, build a new client: {}",
                        error
                    ),
                );
                return;
            }
        };

        vals.disconnected();
        client.end_attempt();
        match end {
            // A panic is reported even if the user disconnected meanwhile.
            SessionEnd::Panicked(task, message) => {
                client.fail(None, format!("Client {} task panicked: {}", task, message))
            }
            // The user's own disconnect has no reason to report.
            SessionEnd::Closed(..) if client.disconnect_requested() => {
                client.disconnected(None, None)
            }
            SessionEnd::Closed(reason, error) => {
                next = retry.disconnected(&client, reason, error);
            }
        }
    }
}

/// How the wait for the server's answer to the handshake ended.
enum Answer {
    /// The server accepted the client; holds its first message.
    Accepted(ServerMessage),
    /// The server refused the client, the connection broke, or no answer came
    /// in time.
    NotAccepted(CloseReason, String),
    /// The user disconnected before the server answered.
    Cancelled,
}

/// Waits for the server's answer to the handshake.
///
/// The server answers an accepted handshake with its synchronization
/// messages and closes the connection of a refused client. `attempt` bounds
/// the wait; a connect-request notice from the server sets a new deadline.
///
/// Until the server accepts the client, the client writes nothing: the
/// server reads a waiting client only to notice that it left. What the UI
/// queues meanwhile belongs to no connection and is dropped, like what it
/// queues while disconnected.
async fn await_answer(
    socket_read: &mut WsClientRead,
    rx: &mut MessageReceiver,
    client: &ClientCore,
    attempt: Attempt,
) -> Answer {
    let Attempt {
        mut deadline,
        timeout,
    } = attempt;
    let mut waiting = false;
    let mut cancelled = pin!(disconnect_requested(rx, client));
    loop {
        let read = until(socket_read.read(), deadline.as_mut());
        let read = match select(pin!(read), cancelled.as_mut()).await {
            Either::Left((read, _)) => read,
            Either::Right(_) => return Answer::Cancelled,
        };
        let Some(read) = read else {
            let error = if waiting {
                "The server did not answer the connect request in time".to_string()
            } else {
                no_answer(timeout)
            };
            return Answer::NotAccepted(CloseReason::Timeout, error);
        };
        match read {
            Ok((ServerHeader::ConnectPending(millis), _)) => {
                waiting = true;
                client.set_state(ConnectionState::WaitingForApproval);
                let wait = Duration::from_millis(millis.into()) + APPROVAL_MARGIN;
                deadline = timer::sleep(wait);
            }
            Ok(message) => return Answer::Accepted(message),
            Err(error) => {
                let (reason, error) = close_cause(error);
                return Answer::NotAccepted(reason, error);
            }
        }
    }
}

/// The error of an attempt the server did not answer within `timeout`.
fn no_answer(timeout: Duration) -> String {
    format!("The server did not answer within {timeout:?}")
}

/// How long a connection attempt may still wait for the server's answer.
struct Attempt {
    deadline: Sleep,
    timeout: Duration,
}

/// How one connection ended.
enum SessionEnd {
    /// An established connection ended, with its reason and a description
    /// when known.
    Closed(Option<CloseReason>, Option<String>),
    /// A connection task panicked; holds the task name and the panic message.
    Panicked(&'static str, String),
}

/// Runs the receive and send tasks of one accepted connection until both
/// end. `first` is the server's first message.
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
    first: ServerMessage,
) -> Result<(MessageReceiver, SessionEnd), String> {
    // A panicking send task stops the receive task; reading alone would keep
    // the connection open while nothing can be sent.
    let (abort_receive, abort_registration) = AbortHandle::new_pair();

    // File transfers run for this connection only. Their control frames share
    // the message queue, so they keep their order with the state messages;
    // file data goes in its own lane, sent only when no message waits.
    let transfers = vals.transfers.clone();
    let lane_sender = sender.clone();
    let generation = transfers.attach(Arc::new(move |message| {
        lane_sender.send(ChannelMessage::Raw(message));
    }));
    let (chunk_tx, chunks) = mpsc::channel(1);
    let pump_future = {
        let transfers = transfers.clone();
        async move {
            // A panicking pump would leave the transfers stuck; fail them.
            if catch_panic(transfers.pump(generation, chunk_tx))
                .await
                .is_err()
            {
                transfers.detach(generation);
            }
        }
    };

    let recv_future = {
        let (vals, client, sender) = (vals.clone(), client.clone(), sender.clone());
        let transfers = transfers.clone();
        async move {
            // A shutdown stops reading at once; the send task still writes the
            // messages queued so far and closes the socket.
            let read = client.until_shutdown(catch_panic(receive(
                socket_read,
                &vals,
                &client,
                &sender,
                first,
            )));
            let end = Abortable::new(read, abort_registration).await;
            // The connection is over: unfinished transfers fail, which also
            // ends the pump.
            transfers.detach(generation);
            // Closing the queue ends the send task.
            sender.close();
            // The read is aborted only after the send task panicked, which is
            // reported instead.
            end.ok()
                .flatten()
                .unwrap_or(Ok(SessionEnd::Closed(None, None)))
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
            let write = catch_panic(send(&mut serializer, socket_send, chunks));
            let end = client.until_flush_deadline(write).await.unwrap_or(Ok(()));
            if end.is_err() {
                abort_receive.abort();
            }
            (serializer.close(), end)
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    let (recv_end, send_end) = {
        let pump_task = tokio::spawn(pump_future);
        let sink_engine = transfers.clone();
        let sink_task = tokio::spawn(async move { sink_engine.sink_worker(generation).await });
        let recv_task = tokio::spawn(recv_future);
        let send_task = tokio::spawn(send_future);
        let recv_end = recv_task.await.unwrap_or_else(|error| {
            // The task stopped before closing the queue.
            sender.close();
            Err(join_error_message(error))
        });
        let send_end = send_task.await.map_err(join_error_message);
        transfers.detach(generation);
        pump_task.abort();
        sink_task.abort();
        (recv_end, send_end?)
    };

    #[cfg(target_arch = "wasm32")]
    let (recv_end, send_end) = {
        let (recv_end, send_end, ()) = tokio::join!(recv_future, send_future, pump_future);
        (recv_end, send_end)
    };

    let (rx, send_end) = send_end;
    let end = match (recv_end, send_end) {
        (Err(message), _) => SessionEnd::Panicked("receive", message),
        (Ok(_), Err(message)) => SessionEnd::Panicked("send", message),
        (Ok(end), Ok(())) => end,
    };
    Ok((rx, end))
}

/// Reads and applies server messages, starting with `message`, until the
/// connection ends.
///
/// The UI is repainted once per frame of messages that changed a state, after
/// all of the frame's messages are applied, so states the server sets one
/// after another reach the UI in one repaint.
async fn receive(
    mut socket_read: ServerReader<impl ServerFrames>,
    vals: &ValuesList,
    client: &ClientCore,
    sender: &MessageSender,
    mut message: ServerMessage,
) -> SessionEnd {
    let mut repaint = false;
    let error = loop {
        repaint |= message.0.changes_state();
        if let Err(e) = handle_message(message, vals, client).await {
            let error = format!("handling message from server failed: {:?}", e);
            sender.send_message(&error);
            print_error(&error);
            // break; TODO: decide if we want to break the loop on error
        }
        if repaint && socket_read.frame_done() {
            client.update(0.);
            repaint = false;
        }
        match socket_read.read().await {
            Ok(next) => message = next,
            Err(e) => break e,
        }
    };

    print_error(&format!("Connection with server failed: {:?}", error));
    let (reason, error) = close_cause(error);
    SessionEnd::Closed(Some(reason), Some(error))
}

/// Why the server's connection ended, and its description.
fn close_cause(error: ReadError) -> (CloseReason, String) {
    match error {
        ReadError::Closed(Some((code, text))) => match CloseReason::from_code(code) {
            Some(reason) if text.is_empty() => (reason, reason.to_string()),
            Some(reason) => (reason, text),
            None if text.is_empty() => (
                CloseReason::ConnectionLost,
                format!("The connection was closed with code {code}"),
            ),
            None => (
                CloseReason::ConnectionLost,
                format!("The connection was closed with code {code}: {text}"),
            ),
        },
        ReadError::Closed(None) => (
            CloseReason::ConnectionLost,
            "The server closed the connection without a reason".to_string(),
        ),
        ReadError::Failed(error) => (CloseReason::ConnectionLost, error.to_string()),
    }
}

/// Writes queued messages until the queue closes or the socket fails, and
/// file data chunks whenever no message waits.
async fn send(
    serializer: &mut MessagesSerializer,
    mut socket_send: WsClientSend,
    mut chunks: mpsc::Receiver<Vec<u8>>,
) {
    let mut chunks_open = true;
    loop {
        // `select` polls the message queue first, so messages win over file
        // data. Both futures only await a channel, so dropping the loser
        // loses nothing.
        let message = if chunks_open {
            match select(pin!(serializer.next()), pin!(chunks.recv())).await {
                Either::Left((message, _)) => message,
                Either::Right((Some(chunk), _)) => Some(FastVec::Heap(chunk)),
                Either::Right((None, _)) => {
                    chunks_open = false;
                    continue;
                }
            }
        } else {
            serializer.next().await
        };
        let Some(message) = message else {
            break;
        };
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
/// A client starts in [`Self::NotConnected`]. Calling [`Client::connect`]
/// starts an attempt in [`Self::Connecting`], which becomes
/// [`Self::Connected`] once the server accepts the handshake, possibly after
/// [`Self::WaitingForApproval`]. An established connection that ends becomes
/// [`Self::Disconnected`]. An attempt that does not reach an established
/// connection, or a connection whose background task panicked, ends in
/// [`Self::Failed`]. A client built with [`ClientBuilder::reconnect`] goes to
/// [`Self::Reconnecting`] instead when the cause can pass, and starts the
/// next attempt by itself. [`Client::last_reason`] and [`Client::last_error`]
/// tell why a connection failed or ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionState {
    /// No connection attempt has been made yet, or the last one was cancelled
    /// with [`Client::disconnect`].
    NotConnected,
    /// A connection attempt is running: the client is opening the connection
    /// or waiting for the server to accept its handshake.
    Connecting,
    /// The server asks its connect-request callback whether to accept this
    /// client, for example by asking the user of the connected client.
    WaitingForApproval,
    /// The server accepted the handshake and state synchronization is active.
    Connected,
    /// A previously active connection ended. [`Client::last_reason`] tells why,
    /// unless the client ended it with [`Client::disconnect`].
    Disconnected,
    /// The server could not be reached, did not accept the handshake, or a
    /// background connection task panicked. [`Client::last_reason`] and
    /// [`Client::last_error`] describe the cause; call [`Client::connect`] to
    /// retry.
    Failed,
    /// An attempt failed or a connection ended, and the client tries again
    /// after a delay ([`ClientBuilder::reconnect`]). [`Client::last_reason`]
    /// and [`Client::last_error`] describe the cause. [`Client::connect`]
    /// starts the attempt at once; [`Client::disconnect`] stops trying.
    Reconnecting,
}

/// Why a connection attempt failed or a connection ended.
///
/// [`Client::last_reason`] returns it after [`ConnectionState::Failed`],
/// [`ConnectionState::Disconnected`] and [`ConnectionState::Reconnecting`]. The server sends the reasons it decides
/// on, so they reach the client also in the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CloseReason {
    /// The server could not be reached: it is not running, the address is
    /// wrong, or it is not a state server.
    Unreachable,
    /// The server did not answer the connection attempt in time.
    Timeout,
    /// The server uses another protocol version; client and server come from
    /// incompatible `egui_states` releases.
    ProtocolMismatch,
    /// The server requires another application version, or one the client
    /// did not send.
    VersionMismatch,
    /// The server rejected the token, or the client sent none.
    TokenRejected,
    /// Another client is connected and the server keeps it.
    Busy,
    /// The server's connect-request callback refused the client, or did not
    /// answer in time.
    Refused,
    /// Another client took over the connection.
    Replaced,
    /// The server closed the connection: it disconnected the client or failed
    /// to synchronize it.
    ClosedByServer,
    /// The server stopped.
    ServerStopped,
    /// The connection broke, or the server closed it without a reason.
    ConnectionLost,
}

impl CloseReason {
    /// The reasons the server's close codes stand for.
    const CODES: [(u16, Self); 8] = [
        (close::PROTOCOL_MISMATCH, Self::ProtocolMismatch),
        (close::VERSION_MISMATCH, Self::VersionMismatch),
        (close::TOKEN_REJECTED, Self::TokenRejected),
        (close::BUSY, Self::Busy),
        (close::REFUSED, Self::Refused),
        (close::REPLACED, Self::Replaced),
        (close::CLOSED_BY_SERVER, Self::ClosedByServer),
        (close::SERVER_STOPPED, Self::ServerStopped),
    ];

    /// The reason the server's close `code` stands for.
    fn from_code(code: u16) -> Option<Self> {
        Self::CODES
            .iter()
            .find(|(known, _)| *known == code)
            .map(|(_, reason)| *reason)
    }
}

impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unreachable => "the server could not be reached",
            Self::Timeout => "the server did not answer in time",
            Self::ProtocolMismatch => "the server uses another protocol version",
            Self::VersionMismatch => "the server requires another application version",
            Self::TokenRejected => "the server rejected the token",
            Self::Busy => "another client is connected",
            Self::Refused => "the server refused the connection",
            Self::Replaced => "another client took over the connection",
            Self::ClosedByServer => "the server closed the connection",
            Self::ServerStopped => "the server stopped",
            Self::ConnectionLost => "the connection was lost",
        })
    }
}

/// The connection state with why it was reached.
struct Status {
    state: ConnectionState,
    /// Set only together with `Failed`, `Disconnected` or `Reconnecting`.
    reason: Option<CloseReason>,
    /// Set only together with `Failed`, `Disconnected` or `Reconnecting`.
    error: Option<String>,
}

struct ClientInner {
    context: Context,
    connect_signal: Event,
    // Set once, when the last `Client` handle is dropped.
    shutdown: Event,
    status: RwLock<Status>,
    /// Set by `Client::disconnect`, so the end of the connection it closes
    /// has no reason to report and no reconnecting follows; cleared by
    /// `Client::connect`.
    closed_by_user: AtomicBool,
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
            status: RwLock::new(Status {
                state: ConnectionState::NotConnected,
                reason: None,
                error: None,
            }),
            closed_by_user: AtomicBool::new(false),
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

    /// Ends a connection attempt or connection: requests to connect made
    /// during it do not start another attempt.
    fn end_attempt(&self) {
        self.0.connect_signal.clear();
    }

    pub(crate) fn set_state(&self, state: ConnectionState) {
        self.set_status(Status {
            state,
            reason: None,
            error: None,
        });
    }

    /// Ends the attempt or connection in `Failed`.
    pub(crate) fn fail(&self, reason: Option<CloseReason>, error: String) {
        print_error(&error);
        self.set_status(Status {
            state: ConnectionState::Failed,
            reason,
            error: Some(error),
        });
    }

    /// Ends the attempt or connection in `Reconnecting`.
    fn reconnecting(&self, reason: Option<CloseReason>, error: Option<String>) {
        if let Some(error) = &error {
            print_error(error);
        }
        self.set_status(Status {
            state: ConnectionState::Reconnecting,
            reason,
            error,
        });
    }

    /// Tells whether the user called `Client::disconnect` after the last call
    /// to `Client::connect`.
    fn disconnect_requested(&self) -> bool {
        self.0.closed_by_user.load(Ordering::Acquire)
    }

    /// Ends an established connection in `Disconnected`.
    fn disconnected(&self, reason: Option<CloseReason>, error: Option<String>) {
        self.set_status(Status {
            state: ConnectionState::Disconnected,
            reason,
            error,
        });
    }

    fn set_status(&self, status: Status) {
        *self.0.status.write() = status;
        self.0.context.request_repaint();
    }

    #[inline]
    pub(crate) fn send_ack(&self, id: u64) {
        self.0.sender.send_ack(id);
    }

    pub(crate) fn send_ack_key(&self, id: u64, key: u32) {
        self.0.sender.send_ack_key(id, key);
    }

    /// Answers a mark of the server's low-priority lane.
    #[inline]
    pub(crate) fn lane_ack(&self) {
        self.0.sender.send(ChannelMessage::LaneAck);
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
    /// It does not block the calling UI thread. A failed attempt waits for
    /// another call to `connect`, unless the client was built with
    /// [`ClientBuilder::reconnect`]; then a call during
    /// [`ConnectionState::Reconnecting`] starts the next attempt at once.
    pub fn connect(&self) {
        let inner = &self.core().0;
        inner.closed_by_user.store(false, Ordering::Release);
        inner.connect_signal.set();
    }

    /// Closes the active connection, cancels a connection attempt, or stops
    /// reconnecting.
    ///
    /// After a closed connection the client reports
    /// [`ConnectionState::Disconnected`] without a reason; after a cancelled
    /// attempt or reconnecting, [`ConnectionState::NotConnected`]. The client
    /// connects again only after [`Self::connect`]. In the browser, a
    /// disconnect while the connection is being opened takes effect once it
    /// is open or has failed.
    pub fn disconnect(&self) {
        self.core().0.closed_by_user.store(true, Ordering::Release);
        self.core().0.sender.close();
    }

    /// Returns the current connection status.
    pub fn get_state(&self) -> ConnectionState {
        self.core().0.status.read().state
    }

    /// Returns why the client is in [`ConnectionState::Failed`],
    /// [`ConnectionState::Disconnected`] or [`ConnectionState::Reconnecting`].
    ///
    /// Returns `None` in every other state, after [`Self::disconnect`], and
    /// after a background task panicked, which [`Self::last_error`] describes.
    /// A new connection attempt clears it.
    pub fn last_reason(&self) -> Option<CloseReason> {
        self.core().0.status.read().reason
    }

    /// Returns a description of why the client is in
    /// [`ConnectionState::Failed`], [`ConnectionState::Disconnected`] or
    /// [`ConnectionState::Reconnecting`].
    ///
    /// The description is meant for people; it includes the server's own
    /// explanation when the server refused or closed the connection. Use
    /// [`Self::last_reason`] to react to the cause. Returns `None` in every
    /// other state and after [`Self::disconnect`]; a new connection attempt
    /// clears it.
    pub fn last_error(&self) -> Option<String> {
        self.core().0.status.read().error.clone()
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
    connect_timeout: Duration,
    reconnect: Option<Reconnect>,
    #[cfg(all(feature = "tls", not(target_arch = "wasm32")))]
    tls: Option<Tls>,
}

impl<T> ClientBuilder<T>
where
    T: State,
{
    /// Creates the state tree and a builder targeting the server on `port`.
    ///
    /// Without [`Self::host`], a native client connects to `127.0.0.1:port`.
    /// A WASM client connects back to the host and port its page was loaded
    /// from, so a page served by the state server reaches it through any
    /// address, tunnel, or forwarded port; `port` is then used
    /// only when the page has no host, such as a page opened from a file.
    /// WASM clients use `wss` when the page was loaded over HTTPS.
    ///
    /// The client requests repaints of `context` when the connection state
    /// changes or the server asks for an update. No application version or
    /// authentication token is set by default, a connection attempt waits
    /// [`DEFAULT_CONNECT_TIMEOUT`] for the server, and the client does not
    /// reconnect by itself. The derived [`State::new`]
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
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            reconnect: None,
            #[cfg(all(feature = "tls", not(target_arch = "wasm32")))]
            tls: None,
        }
    }

    /// Sets the server host: a host name such as `myserver.local`, or an IPv4
    /// or IPv6 address, without scheme or port. An IPv6 address may be in
    /// brackets, as in a URL.
    ///
    /// The client connects to this host on the builder's port, also on WASM,
    /// where it replaces the default of the page's own host and port.
    pub fn host(self, host: impl Into<String>) -> Self {
        let host = host.into();
        Self {
            host: Some(without_brackets(&host).to_string()),
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

    /// Sets how long a connection attempt waits for the server's answer
    /// before it fails with [`CloseReason::Timeout`]. Default
    /// [`DEFAULT_CONNECT_TIMEOUT`].
    ///
    /// On native targets it covers opening the connection and the handshake.
    /// In the browser it starts once the connection is open, since the
    /// browser limits the opening itself. While the server asks its
    /// connect-request callback ([`ConnectionState::WaitingForApproval`]),
    /// the client waits as long as the server says it will take instead.
    pub fn connect_timeout(self, timeout: Duration) -> Self {
        Self {
            connect_timeout: timeout,
            ..self
        }
    }

    /// Connects again by itself after a connection attempt failed or a
    /// connection ended, as `reconnect` says.
    ///
    /// Reconnecting starts with the first call to [`Client::connect`] and
    /// stops with [`Client::disconnect`]. Between attempts the client reports
    /// [`ConnectionState::Reconnecting`]. Each connection synchronizes every
    /// state from the server, which replaces what the UI changed while
    /// disconnected.
    ///
    /// ```no_run
    /// # use egui_states::{ClientBuilder, Reconnect, State, StatesCreator, Value};
    /// # struct AppState { count: Value<i32> }
    /// # impl State for AppState {
    /// #     const NAME: &'static str = "AppState";
    /// #     fn new(c: &mut impl StatesCreator) -> Self { Self { count: c.value("count", 0) } }
    /// # }
    /// # let context = egui::Context::default();
    /// let (state, client) = ClientBuilder::<AppState>::new(context, 8091)
    ///     .reconnect(Reconnect::default())
    ///     .build();
    /// client.connect();
    /// ```
    pub fn reconnect(self, reconnect: Reconnect) -> Self {
        Self {
            reconnect: Some(reconnect),
            ..self
        }
    }

    /// Connects with TLS, which encrypts the connection, checking the server
    /// as `tls` says.
    ///
    /// The server must have TLS turned on. Without this call the client
    /// connects without TLS, and a connection to a server that requires TLS
    /// fails, as does a TLS connection to a server without it; the error is
    /// reported by [`Client::last_error`].
    ///
    /// A WASM client ignores this: the browser connects with TLS when the page
    /// was loaded over HTTPS, and decides itself which certificates to accept.
    #[cfg(feature = "tls")]
    pub fn tls(self, tls: Tls) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Self {
            tls: Some(tls),
            ..self
        };
        #[cfg(target_arch = "wasm32")]
        {
            let _ = tls;
            self
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
            connect_timeout,
            reconnect,
            #[cfg(all(feature = "tls", not(target_arch = "wasm32")))]
            tls,
        } = self;

        #[cfg(all(feature = "tls", not(target_arch = "wasm32")))]
        let secure = tls.is_some();
        #[cfg(not(all(feature = "tls", not(target_arch = "wasm32"))))]
        let secure = false;
        let url = server_url(host.as_deref(), port, secure);
        let values = creator.get_values();
        #[cfg(target_arch = "wasm32")]
        {
            use crate::client::states::file_transfer::{download_base, start_browser_download};
            let base = download_base(&url);
            values.transfers.set_ticket_hook(Arc::new(move |download| {
                let path = crate::file_transfer::engine::DOWNLOAD_PATH;
                start_browser_download(&format!("{base}{path}{}", download.ticket), &download.name);
            }));
        }
        let target = Target {
            url,
            #[cfg(not(target_arch = "wasm32"))]
            host: host.unwrap_or_else(|| "127.0.0.1".to_string()),
            #[cfg(not(target_arch = "wasm32"))]
            port,
            #[cfg(all(feature = "tls", not(target_arch = "wasm32")))]
            tls,
        };
        let client = ClientCore::new(context, sender.clone());
        let client_out = Client::new(client.clone());
        let settings = Settings {
            target,
            version,
            token,
            connect_timeout,
            reconnect,
        };

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
                runtime.block_on(start_gui_client(settings, values, rx, sender, client))
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                start_gui_client(settings, values, rx, sender, client).await;
            });
        }

        (states, client_out)
    }
}

/// Removes the brackets around an IPv6 address written as in a URL. The
/// address is kept without them, which is how a connection is opened to it.
fn without_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .filter(|address| address.parse::<Ipv6Addr>().is_ok())
        .unwrap_or(host)
}

/// Formats `host` for a URL, bracketing an IPv6 address.
fn url_host(host: &str) -> String {
    match host.parse::<Ipv6Addr>() {
        Ok(_) => format!("[{host}]"),
        Err(_) => host.to_string(),
    }
}

/// Returns the WebSocket URL of the server, `wss` when `secure`.
#[cfg(not(target_arch = "wasm32"))]
fn server_url(host: Option<&str>, port: u16, secure: bool) -> String {
    let scheme = if secure { "wss" } else { "ws" };
    format!(
        "{scheme}://{}:{port}/ws",
        url_host(host.unwrap_or("127.0.0.1"))
    )
}

/// Returns the WebSocket URL of the server.
///
/// Without an explicit host this is the page's own host and port, read once
/// when the client is built. The scheme follows the page's, as browsers
/// require; `ClientBuilder::tls` does not change it.
#[cfg(target_arch = "wasm32")]
fn server_url(host: Option<&str>, port: u16, _secure: bool) -> String {
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
        assert_eq!(server_url(None, 8091, false), "ws://127.0.0.1:8091/ws");
        assert_eq!(
            server_url(Some("myserver.local"), 8091, false),
            "ws://myserver.local:8091/ws"
        );
        assert_eq!(
            server_url(Some("10.0.0.2"), 80, false),
            "ws://10.0.0.2:80/ws"
        );
        assert_eq!(server_url(Some("::1"), 8091, false), "ws://[::1]:8091/ws");
        assert_eq!(server_url(Some("::1"), 8091, true), "wss://[::1]:8091/ws");
    }

    #[test]
    fn ipv6_hosts_are_accepted_with_and_without_brackets() {
        for (host, expected) in [
            ("::1", "::1"),
            ("[::1]", "::1"),
            ("[fe80::1]", "fe80::1"),
            ("myserver.local", "myserver.local"),
            ("10.0.0.2", "10.0.0.2"),
            // Only an address is unwrapped.
            ("[myserver]", "[myserver]"),
        ] {
            assert_eq!(without_brackets(host), expected);
            let builder = ClientBuilder::<Empty>::new(Context::default(), 8091).host(host);
            assert_eq!(builder.host.as_deref(), Some(expected));
        }
        assert_eq!(
            server_url(Some(without_brackets("[::1]")), 8091, false),
            "ws://[::1]:8091/ws"
        );
    }

    struct Empty;

    impl State for Empty {
        const NAME: &'static str = "Empty";

        fn new(_: &mut impl crate::StatesCreator) -> Self {
            Self
        }
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

    /// Runs one egui pass and drops its texture changes, as a UI would apply
    /// them.
    fn run_pass(context: &egui::Context) {
        let mut output = context.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
    }

    /// Frames queued in advance; each read first runs an egui pass, as the
    /// UI would between frames, so every repaint request is observable.
    /// `in_pass` is set during the pass, whose own repaint requests (egui
    /// follows a requested pass with one more) are not the client's.
    struct FramesBetweenPasses {
        context: egui::Context,
        in_pass: Arc<std::sync::atomic::AtomicBool>,
        frames: std::collections::VecDeque<bytes::Bytes>,
    }

    impl ServerFrames for FramesBetweenPasses {
        async fn next_frame(&mut self) -> Result<bytes::Bytes, ReadError> {
            self.in_pass.store(true, Ordering::SeqCst);
            run_pass(&self.context);
            self.in_pass.store(false, Ordering::SeqCst);
            self.frames.pop_front().ok_or(ReadError::Closed(None))
        }
    }

    /// The client repaints once per frame that changed a state, after all of
    /// the frame's messages are applied, and not after a frame that changed
    /// none.
    #[test]
    fn the_client_repaints_once_per_frame_that_changed_a_state() {
        use crate::client::states_creator::StatesCreatorClient;
        use crate::protocol::serialize;
        use crate::{StatesCreator, Typed};

        let context = egui::Context::default();
        for _ in 0..3 {
            run_pass(&context);
        }
        let (sender, _receiver) = MessageSender::new();
        let mut creator = StatesCreatorClient::new(sender.clone(), context.clone(), "root".into());
        let status: crate::Static<i32> = creator.add_static("status", 0);
        let values = creator.get_values();
        let id = *values.states.keys().next().unwrap();
        let type_id = <i32 as Typed>::get_type().get_hash();

        // Records the value the UI would show at each repaint request.
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let in_pass = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (record, shown, passing) = (seen.clone(), status.clone(), in_pass.clone());
        context.set_request_repaint_callback(move |_| {
            if !passing.load(Ordering::SeqCst) {
                record.lock().push(shown.get());
            }
        });

        let set = |value: i32| {
            let payload = postcard::to_stdvec(&value).unwrap();
            let header = ServerHeader::Static(id, type_id, payload.len() as u32);
            let mut message = serialize::<_, 32>(&header).unwrap().into_bytes().to_vec();
            message.extend_from_slice(&payload);
            message
        };
        let lane_mark = serialize::<_, 32>(&ServerHeader::LaneMark)
            .unwrap()
            .into_bytes()
            .to_vec();
        let frames = [[set(1), set(2)].concat(), lane_mark, set(3)]
            .map(bytes::Bytes::from)
            .into();
        let mut reader = ServerReader::new(FramesBetweenPasses {
            context: context.clone(),
            in_pass,
            frames,
        });
        let client = ClientCore::new(context, sender.clone());

        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let first = reader.read().await.unwrap();
            receive(reader, &values, &client, &sender, first).await;
        });
        assert_eq!(*seen.lock(), [2, 3]);
        assert_eq!(status.get(), 3);
    }

    /// Unreachable servers and lost connections are retried with waits that
    /// double up to the longest, a busy server after its own wait, and the
    /// other causes not at all.
    #[test]
    fn reconnecting_retries_only_causes_that_can_pass() {
        let policy = Reconnect {
            delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(350),
            busy_delay: Duration::from_secs(2),
        };
        let mut retry = Retry::new(Some(policy));
        let millis = |retry: &mut Retry, reason| retry.after(Some(reason)).map(|d| d.as_millis());
        assert_eq!(millis(&mut retry, CloseReason::Unreachable), Some(100));
        assert_eq!(millis(&mut retry, CloseReason::Timeout), Some(200));
        assert_eq!(millis(&mut retry, CloseReason::Busy), Some(2000));
        assert_eq!(millis(&mut retry, CloseReason::ConnectionLost), Some(350));
        assert_eq!(millis(&mut retry, CloseReason::ServerStopped), Some(350));
        for reason in [
            CloseReason::ProtocolMismatch,
            CloseReason::VersionMismatch,
            CloseReason::TokenRejected,
            CloseReason::Refused,
            CloseReason::Replaced,
            CloseReason::ClosedByServer,
        ] {
            assert_eq!(retry.after(Some(reason)), None, "{reason:?}");
        }
        // A panic has no reason and is not retried either.
        assert_eq!(retry.after(None), None);
        retry.reset();
        assert_eq!(millis(&mut retry, CloseReason::Unreachable), Some(100));

        let mut never = Retry::new(None);
        assert_eq!(never.after(Some(CloseReason::Unreachable)), None);
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

        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
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
