use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, Interval, MissedTickBehavior, interval_at, timeout_at};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use crate::PROTOCOL_VERSION;
use crate::event::Event;
use crate::serialization::{
    MAX_MSG_COUNT, MSG_SIZE_THRESHOLD, ServerHeader, WEBSOCKET_MAX_SIZE, serialize,
};
use crate::server_core::heartbeat::{Activity, Heartbeat};
use crate::server_core::origin::OriginPolicy;
use crate::server_core::sender::{MessageReceiver, MessageSender, SenderData};
use crate::server_core::server::ServerStatesList;
use crate::server_core::signals::SignalsManager;
use crate::server_core::socket_reader::{ClientMessage, SocketReader};
use crate::server_core::web::{self, Dispatch, WebSite, WebSlot};

enum ChannelHolder {
    Transfer(JoinHandle<MessageReceiver>),
    Rx(MessageReceiver),
}

#[derive(Clone)]
pub(crate) struct Handshake {
    pub version: Option<u64>,
    pub token: Option<String>,
    /// Browser pages allowed to open the WebSocket.
    pub origins: Arc<OriginPolicy>,
}

/// Bounds how long a new connection may take to send its request head and
/// complete the WebSocket and protocol handshakes.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounds the connections that are handshaking or receiving the web page at
/// once. Further connections wait in the operating system's accept queue until
/// one finishes, which caps the memory and file descriptors that unauthenticated
/// peers can hold. Browsers open about six connections per host.
pub(crate) const MAX_PENDING_CONNECTIONS: usize = 32;

/// Pause after a failed accept. Errors such as running out of file descriptors
/// repeat until a connection closes, so retrying at once would spin.
const ACCEPT_ERROR_DELAY: Duration = Duration::from_millis(100);

/// A connection that passed the protocol handshake and may replace the
/// current client.
struct ClientReady {
    socket_tx: SplitSink<WebSocketStream<TcpStream>, Message>,
    socket_reader: SocketReader,
    peer_addr: String,
}

/// What the run loop shares with the tasks of each connection.
#[derive(Clone)]
pub(crate) struct Session {
    pub(crate) sender: MessageSender,
    pub(crate) connected: Arc<AtomicBool>,
    pub(crate) values: ServerStatesList,
    pub(crate) signals: SignalsManager,
    pub(crate) heartbeat: Option<Heartbeat>,
}

pub(crate) async fn run(
    listener: TcpListener,
    rx: MessageReceiver,
    stop_event: Event,
    handshake: Handshake,
    web: WebSlot,
    session: Session,
) -> MessageReceiver {
    let Session {
        sender,
        connected,
        values,
        signals,
        ..
    } = session.clone();
    let mut holder = ChannelHolder::Rx(rx);
    // Connections still handshaking or receiving the web page. They run
    // concurrently so an idle connection cannot hold up the others; dropping
    // the set on shutdown aborts them. At most `MAX_PENDING_CONNECTIONS` run at
    // once.
    let mut pending: JoinSet<Option<ClientReady>> = JoinSet::new();

    loop {
        // check for stop event, a finished handshake or incoming connection
        let client = tokio::select! {
            biased;
            _ = stop_event.wait_clear_async() => {
                break;
            }
            Some(result) = pending.join_next(), if !pending.is_empty() => {
                match result {
                    Ok(Some(client)) => client,
                    Ok(None) => continue,
                    Err(e) => {
                        if e.is_panic() {
                            signals.error("connection task panicked");
                        }
                        continue;
                    }
                }
            }
            s = listener.accept(), if pending.len() < MAX_PENDING_CONNECTIONS => {
                match s {
                    Ok((stream, _)) => {
                        if let Err(e) = stream.set_nodelay(true) {
                            signals.error(format!("failed to set TCP_NODELAY: {:?}", e));
                            continue;
                        }
                        let site = web.read().clone();
                        pending.spawn(accept_client(
                            stream,
                            site,
                            handshake.clone(),
                            signals.clone(),
                        ));
                    }
                    Err(e) => {
                        signals.error(format!("accepting connection failed: {:?}", e));
                        tokio::time::sleep(ACCEPT_ERROR_DELAY).await;
                    }
                }
                continue;
            }
        };

        let ClientReady {
            socket_tx,
            socket_reader,
            peer_addr,
        } = client;

        let mut rx = match holder {
            // disconnect previous client
            ChannelHolder::Transfer(handler) => {
                #[cfg(debug_assertions)]
                signals.debug("terminating previous connection");
                connected.store(false, Ordering::Release);
                sender.close();
                handler.await.expect("joining communication handler failed")
            }
            ChannelHolder::Rx(rx) => rx,
        };

        // clean mesage queue and send sync signals
        while !rx.is_empty() {
            let _ = rx.recv().await;
        }

        connected.store(true, Ordering::Release);
        let failure = if values.sync.iter().any(|v| v.sync().is_err()) {
            Some("failed to sync value after handshake")
        } else {
            match serialize(&ServerHeader::Update(0.0)) {
                Ok(data) => {
                    sender.send(data);
                    None
                }
                Err(_) => Some("failed to serialize update message"),
            }
        };
        if let Some(error) = failure {
            // Drop only this client; the server keeps accepting connections.
            // The partial sync may have left producers waiting for this
            // client's acknowledgements, so release them.
            signals.error(error);
            connected.store(false, Ordering::Release);
            for v in values.ack.values() {
                v.reset();
            }
            holder = ChannelHolder::Rx(rx);
            continue;
        }

        signals.on_connect(peer_addr);

        let activity = socket_reader.activity();
        let reader_handler = tokio::spawn(reader(socket_reader, session.clone()));
        let handler = tokio::spawn(writer(
            rx,
            socket_tx,
            reader_handler,
            activity,
            session.clone(),
        ));

        holder = ChannelHolder::Transfer(handler);
    }

    match holder {
        // disconnect previous client
        ChannelHolder::Transfer(handler) => {
            #[cfg(debug_assertions)]
            signals.debug("terminating previous connection");
            connected.store(false, Ordering::Release);
            sender.close();
            handler.await.expect("joining communication handler failed")
        }
        ChannelHolder::Rx(rx) => rx,
    }
}

/// Takes a new connection up to a passed protocol handshake.
///
/// While the web page is served, plain HTTP requests are answered here and
/// yield `None`. Failures never touch the current client's connection.
async fn accept_client(
    stream: TcpStream,
    site: Option<Arc<WebSite>>,
    handshake: Handshake,
    signals: SignalsManager,
) -> Option<ClientReady> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;

    let stream = match site {
        None => stream,
        Some(site) => match timeout_at(deadline, web::dispatch(stream)).await {
            Ok(Ok(Dispatch::WebSocket(stream))) => stream,
            Ok(Ok(Dispatch::Http(stream, request))) => {
                // Browsers abort downloads on navigation, so failures here are
                // not server errors.
                if let Err(_e) = web::serve(stream, request, &site).await {
                    #[cfg(debug_assertions)]
                    signals.debug(format!("serving web page failed: {:?}", _e));
                }
                return None;
            }
            // Browsers open speculative connections that may stay idle and be
            // closed unused, so these are not reported.
            Ok(Ok(Dispatch::Closed)) | Err(_) => return None,
            Ok(Err(e)) => {
                signals.error(format!("reading request failed: {:?}", e));
                return None;
            }
        },
    };

    match timeout_at(deadline, handshake_client(stream, &handshake, &signals)).await {
        Ok(client) => client,
        Err(_) => {
            signals.warning("client handshake timed out");
            None
        }
    }
}

async fn handshake_client(
    stream: TcpStream,
    handshake: &Handshake,
    signals: &SignalsManager,
) -> Option<ClientReady> {
    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(WEBSOCKET_MAX_SIZE);
    websocket_config.max_frame_size = Some(WEBSOCKET_MAX_SIZE);
    // Refuse the upgrade of a page that may not connect before the protocol
    // handshake, so it never reaches the token check or the current client.
    let mut rejection = None;
    // tungstenite fixes the callback's error type.
    #[allow(clippy::result_large_err)]
    let check_origin = |request: &Request, response: Response| {
        handshake
            .origins
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
    let (socket_tx, socket_rx) = websocket.split();
    let mut socket_reader = SocketReader::new(socket_rx);

    match socket_reader.next().await {
        Err(e) => {
            if let Some(e) = e {
                signals.error(e);
            }
            None
        }
        Ok(ClientMessage::Handshake(p, v, h)) => {
            if let Err(e) = check_handshake(handshake, p, v, h) {
                signals.warning(&e);
                return None;
            }
            Some(ClientReady {
                socket_tx,
                socket_reader,
                peer_addr,
            })
        }
        Ok(_) => {
            signals.error("expected handshake message from client");
            None
        }
    }
}

fn check_handshake(
    server_handshake: &Handshake,
    client_protocol: u16,
    version: Option<u64>,
    token: Option<String>,
) -> Result<(), String> {
    if client_protocol != PROTOCOL_VERSION {
        let message = format!(
            "attempted to connect with wrong protocol version: expected {}, got {}",
            PROTOCOL_VERSION, client_protocol
        );
        return Err(message);
    }

    if let Some(server_version) = server_handshake.version {
        match version {
            Some(client_version) => {
                if client_version != server_version {
                    let message = format!(
                        "attempted to connect with wrong version: expected {}, got {}",
                        server_version, client_version
                    );
                    return Err(message);
                }
            }
            _ => return Err("client version missing".to_string()),
        }
    }

    if let Some(server_token) = &server_handshake.token {
        match token {
            Some(client_token) => {
                if !tokens_match(&client_token, server_token) {
                    return Err("attempted to connect with wrong token".to_string());
                }
            }
            _ => return Err("client token missing".to_string()),
        }
    }

    Ok(())
}

/// Compares tokens in constant time, so the time taken does not reveal how
/// much of a wrong token matched. Only a length mismatch returns early.
fn tokens_match(client_token: &str, server_token: &str) -> bool {
    let (client, server) = (client_token.as_bytes(), server_token.as_bytes());
    if client.len() != server.len() {
        return false;
    }
    let difference = client
        .iter()
        .zip(server)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    std::hint::black_box(difference) == 0
}

async fn reader(mut socket_rx: SocketReader, session: Session) {
    let Session {
        sender,
        connected,
        values,
        signals,
        ..
    } = session;
    loop {
        // read the message
        let result_message = socket_rx.next().await;

        // check if not connected
        if !connected.load(Ordering::Acquire) {
            #[cfg(debug_assertions)]
            signals.debug("read thread is closing");
            signals.reset();
            break;
        }

        match result_message {
            Err(e) => {
                if let Some(e) = e {
                    signals.error(e);
                }
                connected.store(false, Ordering::Release);
                signals.reset();
                break;
            }
            Ok(ClientMessage::Ack(id)) => {
                let val_res = values.ack.get(&id);
                match val_res {
                    Some(val) => {
                        val.acknowledge();
                    }
                    None => {
                        signals.error(format!("value with id {} not found for Acknowledge", id))
                    }
                }
            }
            Ok(ClientMessage::Value(id, type_id, signal, data)) => match values.values.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_value(type_id, signal, data) {
                        signals.error(format!("value updating failed: {}", e));
                    }
                }
                None => signals.error(format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Signal(id, type_id, data)) => match values.signals.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_signal(type_id, data) {
                        signals.error(format!("signal updating failed: {}", e));
                    }
                }
                None => signals.error(format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Message(data)) => {
                signals.client_message(data);
            }
            Ok(ClientMessage::Handshake(_, _, _)) => {
                signals.error("unexpected handshake message after connection established");
            }
        }
    }

    // send close signal to writing thread if reading fails
    #[cfg(debug_assertions)]
    signals.debug("terminating write thread");
    sender.close();
}

async fn writer(
    rx: MessageReceiver,
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
    reader_handle: tokio::task::JoinHandle<()>,
    activity: Arc<Activity>,
    session: Session,
) -> MessageReceiver {
    let Session {
        connected,
        values,
        signals,
        heartbeat,
        ..
    } = session;
    enum Next {
        Message(Option<SenderData>),
        Ping,
    }

    let mut data_receiver = DataReceiver::new(rx);
    // The connection counts as heard from when it starts, however long the
    // initial synchronization took.
    activity.touch();
    let mut pings = heartbeat.map(|heartbeat| {
        let mut pings = interval_at(Instant::now() + heartbeat.interval, heartbeat.interval);
        pings.set_missed_tick_behavior(MissedTickBehavior::Delay);
        pings
    });

    loop {
        // Both branches are cancel safe: `DataReceiver::next` only awaits the
        // channel, and a missed tick is delivered by the next poll.
        let next = tokio::select! {
            message = data_receiver.next() => Next::Message(message),
            () = next_ping(&mut pings) => Next::Ping,
        };

        let message = match next {
            Next::Message(Some(message)) => {
                // if not connected, stop thread
                if !connected.load(Ordering::Acquire) {
                    unless_silent(websocket.close(), heartbeat, &activity).await;
                    break;
                }
                Message::Binary(message.into_bytes())
            }
            // check if message is terminate signal
            Next::Message(None) => {
                signals.info("writer is closing connection");
                unless_silent(websocket.close(), heartbeat, &activity).await;
                break;
            }
            Next::Ping => {
                if let Some(heartbeat) = heartbeat
                    && activity.silent_for() >= heartbeat.timeout
                {
                    report_silent(&signals, heartbeat);
                    break;
                }
                Message::Ping(Bytes::new())
            }
        };

        // A dead client stops reading, so a send can wait for buffer space
        // forever; the heartbeat also ends such a send.
        match unless_silent(websocket.send(message), heartbeat, &activity).await {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                signals.error(format!("sending message to client failed: {:?}", e));
                break;
            }
            None => {
                if let Some(heartbeat) = heartbeat {
                    report_silent(&signals, heartbeat);
                }
                break;
            }
        }
    }

    reader_handle.abort();
    let _ = reader_handle.await;
    // A reader that ends the connection itself drops the client's pending
    // changes; an aborted one never gets there, so their callbacks would
    // otherwise run after the disconnect.
    signals.reset();

    connected.store(false, Ordering::Release);
    signals.on_disconnect();

    // reset all pendings values
    for v in values.ack.values() {
        v.reset();
    }

    data_receiver.finalize()
}

/// Completes at the next heartbeat ping, or never without a heartbeat.
async fn next_ping(pings: &mut Option<Interval>) {
    match pings {
        Some(pings) => {
            pings.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// Runs `operation` to completion, or with a heartbeat until the client has
/// been silent for the heartbeat timeout; `None` means the client went silent.
async fn unless_silent<T>(
    operation: impl Future<Output = T>,
    heartbeat: Option<Heartbeat>,
    activity: &Activity,
) -> Option<T> {
    match heartbeat {
        None => Some(operation.await),
        Some(heartbeat) => tokio::select! {
            biased;
            output = operation => Some(output),
            () = activity.expired(heartbeat.timeout) => None,
        },
    }
}

fn report_silent(signals: &SignalsManager, heartbeat: Heartbeat) {
    signals.warning(format!(
        "client sent nothing for {:?}, disconnecting it",
        heartbeat.timeout
    ));
}

// A helper struct to receive from MessageReceiver and create micro-batches
/// Messages larger than this are not merged with others into one frame:
/// appending would copy them again, and they fill a frame on their own.
const MERGE_LIMIT: usize = 64 * 1024;

struct DataReceiver {
    rx: MessageReceiver,
    send_next: Option<(SenderData, bool)>,
    is_closed: bool,
}

impl DataReceiver {
    fn new(rx: MessageReceiver) -> Self {
        Self {
            rx,
            send_next: None,
            is_closed: false,
        }
    }

    async fn next(&mut self) -> Option<SenderData> {
        let (mut msg, send_now) = match self.send_next.take() {
            Some(next) => next,
            None if self.is_closed => return None,
            None => match self.rx.recv().await {
                Some(Some(next)) => next,
                None | Some(None) => return None,
            },
        };

        if send_now || msg.len() > MERGE_LIMIT {
            return Some(msg);
        }

        let mut counter = 1;
        loop {
            if msg.len() > MSG_SIZE_THRESHOLD || counter >= MAX_MSG_COUNT {
                return Some(msg);
            }

            match self.rx.try_recv() {
                Ok(Some((next_msg, true))) => {
                    self.send_next = Some((next_msg, true));
                    return Some(msg);
                }
                Ok(Some((next_msg, false))) => {
                    if next_msg.len() > MERGE_LIMIT
                        || msg
                            .len()
                            .checked_add(next_msg.len())
                            .is_none_or(|size| size > MSG_SIZE_THRESHOLD)
                    {
                        self.send_next = Some((next_msg, false));
                        return Some(msg);
                    }
                    msg.extend_from_data(&next_msg);
                    counter += 1;
                }
                Err(TryRecvError::Empty) => return Some(msg),
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.is_closed = true;
                    return Some(msg);
                }
            }
        }
    }

    fn finalize(self) -> MessageReceiver {
        self.rx
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::{DataReceiver, MERGE_LIMIT};
    use crate::serialization::{FastVec, MAX_MSG_COUNT, MSG_SIZE_THRESHOLD};

    fn data(size: usize) -> FastVec<32> {
        FastVec::Heap((0..size).map(|i| (i % 251) as u8).collect())
    }

    async fn frames(messages: Vec<(FastVec<32>, bool)>) -> Vec<Vec<u8>> {
        let (tx, rx) = unbounded_channel();
        for message in messages {
            tx.send(Some(message)).unwrap();
        }
        drop(tx);
        let mut receiver = DataReceiver::new(rx);
        let mut frames = Vec::new();
        while let Some(frame) = receiver.next().await {
            frames.push(frame.into_bytes().to_vec());
        }
        frames
    }

    fn bytes(size: usize) -> Vec<u8> {
        data(size).into_bytes().to_vec()
    }

    fn joined(sizes: &[usize]) -> Vec<u8> {
        sizes.iter().flat_map(|size| bytes(*size)).collect()
    }

    #[tokio::test]
    async fn data_receiver_batches_messages_up_to_the_merge_limit() {
        let frames = frames(vec![(data(MERGE_LIMIT), false), (data(10), false)]).await;
        assert_eq!(frames, [joined(&[MERGE_LIMIT, 10])]);
    }

    /// A message above the merge limit is never copied into another frame,
    /// before or after the messages around it.
    #[tokio::test]
    async fn data_receiver_sends_large_messages_alone() {
        let frames = frames(vec![
            (data(5), false),
            (data(MERGE_LIMIT + 1), false),
            (data(6), false),
            (data(7), false),
        ])
        .await;
        assert_eq!(frames, [bytes(5), bytes(MERGE_LIMIT + 1), joined(&[6, 7])]);
    }

    #[tokio::test]
    async fn data_receiver_does_not_batch_past_size_threshold() {
        let frames = frames(vec![
            (data(MSG_SIZE_THRESHOLD - 10), false),
            (data(11), false),
        ])
        .await;
        assert_eq!(frames, [bytes(MSG_SIZE_THRESHOLD - 10), bytes(11)]);
    }

    #[tokio::test]
    async fn data_receiver_keeps_immediate_message_separate() {
        let (tx, rx) = unbounded_channel();
        tx.send(Some((data(10), false))).unwrap();
        tx.send(Some((data(11), true))).unwrap();
        tx.send(Some((data(5), false))).unwrap();
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(10).into_bytes()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(11).into_bytes()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(5).into_bytes()
        );
        assert!(receiver.next().await.is_none());
    }

    #[tokio::test]
    async fn data_receiver_respects_message_count_limit() {
        let (tx, rx) = unbounded_channel();
        for marker in 0..MAX_MSG_COUNT + 2 {
            tx.send(Some((FastVec::Heap(vec![marker as u8]), false)))
                .unwrap();
        }
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            (0..MAX_MSG_COUNT as u8).collect::<Vec<_>>()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            [MAX_MSG_COUNT as u8, MAX_MSG_COUNT as u8 + 1]
        );
        assert!(receiver.next().await.is_none());
    }
}

#[cfg(test)]
mod run_tests {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::atomic::AtomicUsize;

    use parking_lot::RwLock;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio_tungstenite::MaybeTlsStream;

    use super::*;
    use crate::serialization::{ClientHeader, FastVec};
    use crate::server_core::server::{Acknowledge, SyncTrait};

    /// Fails only its first connection sync.
    struct FailFirstSync(AtomicUsize);

    impl SyncTrait for FailFirstSync {
        fn sync(&self) -> Result<(), ()> {
            match self.0.fetch_add(1, Ordering::AcqRel) {
                0 => Err(()),
                _ => Ok(()),
            }
        }
    }

    struct ResetProbe(AtomicBool);

    impl Acknowledge for ResetProbe {
        fn acknowledge(&self) {}

        fn reset(&self) {
            self.0.store(true, Ordering::Release);
        }
    }

    async fn connect(addr: SocketAddr) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        send_handshake(&mut ws).await;
        ws
    }

    async fn send_handshake<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) {
        let handshake = ClientHeader::Handshake(PROTOCOL_VERSION, None, None);
        let bytes = postcard::to_stdvec(&handshake).unwrap();
        ws.send(Message::Binary(bytes.into())).await.unwrap();
    }

    #[test]
    fn failed_sync_drops_only_that_client() {
        if crate::test_support::isolated() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (sender, rx) = MessageSender::new();
            let connected = Arc::new(AtomicBool::new(false));
            let stop_event = Event::new();
            let probe = Arc::new(ResetProbe(AtomicBool::new(false)));
            let mut values = ServerStatesList::default();
            values.ack.insert(1, probe.clone());
            values
                .sync
                .push(Arc::new(FailFirstSync(AtomicUsize::new(0))));
            let server = tokio::spawn(run(
                listener,
                rx,
                stop_event.clone(),
                Handshake {
                    version: None,
                    token: None,
                    origins: Default::default(),
                },
                Arc::new(RwLock::new(None)),
                Session {
                    sender: sender.clone(),
                    connected: connected.clone(),
                    values,
                    signals: SignalsManager::new(),
                    heartbeat: None,
                },
            ));
            let deadline = Duration::from_secs(5);

            let mut rejected = connect(addr).await;
            let closed = tokio::time::timeout(deadline, rejected.next())
                .await
                .expect("the client whose sync failed was not dropped");
            assert!(!matches!(closed, Some(Ok(Message::Binary(_)))));
            assert!(
                probe.0.load(Ordering::Acquire),
                "pending states were not reset"
            );
            assert!(!connected.load(Ordering::Acquire));

            let mut accepted = connect(addr).await;
            let update = tokio::time::timeout(deadline, accepted.next())
                .await
                .expect("the next client was not synchronized");
            assert!(matches!(update, Some(Ok(Message::Binary(_)))));
            assert!(connected.load(Ordering::Acquire));

            connected.store(false, Ordering::Release);
            stop_event.set();
            sender.close();
            tokio::time::timeout(deadline, server)
                .await
                .expect("the server did not stop")
                .unwrap();
        });
    }

    const TEST_HEARTBEAT: Heartbeat = Heartbeat {
        interval: Duration::from_millis(100),
        timeout: Duration::from_millis(400),
    };
    const DEADLINE: Duration = Duration::from_secs(5);

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// Polls `condition` until it holds; `false` if it did not within `within`.
    async fn until(within: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + within;
        while !condition() {
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        true
    }

    /// The connection loop with one pending state that records its reset.
    struct TestServer {
        addr: SocketAddr,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        stop_event: Event,
        probe: Arc<ResetProbe>,
        handle: JoinHandle<MessageReceiver>,
    }

    impl TestServer {
        async fn start(heartbeat: Option<Heartbeat>) -> Self {
            Self::start_with(heartbeat, SignalsManager::new()).await
        }

        async fn start_with(heartbeat: Option<Heartbeat>, signals: SignalsManager) -> Self {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (sender, rx) = MessageSender::new();
            let connected = Arc::new(AtomicBool::new(false));
            let stop_event = Event::new();
            let probe = Arc::new(ResetProbe(AtomicBool::new(false)));
            let mut values = ServerStatesList::default();
            values.ack.insert(1, probe.clone());
            let handle = tokio::spawn(run(
                listener,
                rx,
                stop_event.clone(),
                Handshake {
                    version: None,
                    token: None,
                    origins: Default::default(),
                },
                Arc::new(RwLock::new(None)),
                Session {
                    sender: sender.clone(),
                    connected: connected.clone(),
                    values,
                    signals,
                    heartbeat,
                },
            ));
            Self {
                addr,
                sender,
                connected,
                stop_event,
                probe,
                handle,
            }
        }

        fn connected(&self) -> bool {
            self.connected.load(Ordering::Acquire)
        }

        fn released(&self) -> bool {
            self.probe.0.load(Ordering::Acquire)
        }

        async fn stop(self) {
            self.connected.store(false, Ordering::Release);
            self.stop_event.set();
            self.sender.close();
            tokio::time::timeout(DEADLINE, self.handle)
                .await
                .expect("the server did not stop")
                .unwrap();
        }
    }

    #[test]
    fn heartbeat_disconnects_a_client_that_stops_reading() {
        if crate::test_support::isolated() {
            return;
        }
        block_on(async {
            let server = TestServer::start(Some(TEST_HEARTBEAT)).await;
            // The client never polls its socket, so it never answers a ping.
            let _client = connect(server.addr).await;
            let connected_at = Instant::now();
            assert!(
                until(DEADLINE, || server.connected()).await,
                "the client was not accepted"
            );

            assert!(
                until(DEADLINE, || !server.connected()).await,
                "the silent client was not disconnected"
            );
            assert!(
                connected_at.elapsed() >= TEST_HEARTBEAT.timeout / 2,
                "the client was disconnected before the timeout"
            );
            assert!(server.released(), "pending states were not released");
            server.stop().await;
        });
    }

    #[cfg(feature = "server")]
    #[test]
    fn heartbeat_disconnect_drops_pending_client_changes() {
        if crate::test_support::isolated() {
            return;
        }
        block_on(async {
            const STATE_ID: u64 = 100;
            let signals = SignalsManager::new();
            signals.set_register(STATE_ID, true, false);
            let server = TestServer::start_with(Some(TEST_HEARTBEAT), signals.clone()).await;
            let _client = connect(server.addr).await;
            assert!(
                until(DEADLINE, || server.connected()).await,
                "the client was not accepted"
            );

            // A change from this client that no callback has taken yet.
            signals.set(STATE_ID, Bytes::from_static(b"stale"));
            assert!(
                until(DEADLINE, || !server.connected()).await,
                "the silent client was not disconnected"
            );

            assert!(
                signals.try_claim(None).is_none(),
                "a change from the expired connection is still pending"
            );
            server.stop().await;
        });
    }

    #[test]
    fn heartbeat_keeps_a_client_that_answers_pings() {
        if crate::test_support::isolated() {
            return;
        }
        block_on(async {
            let server = TestServer::start(Some(TEST_HEARTBEAT)).await;
            let mut client = connect(server.addr).await;
            assert!(
                until(DEADLINE, || server.connected()).await,
                "the client was not accepted"
            );

            // Reading lets tungstenite answer every ping; the client sends
            // nothing else.
            let pings = Arc::new(AtomicUsize::new(0));
            let reader = tokio::spawn({
                let pings = pings.clone();
                async move {
                    while let Some(Ok(message)) = client.next().await {
                        if matches!(message, Message::Ping(_)) {
                            pings.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
            tokio::time::sleep(TEST_HEARTBEAT.timeout * 3).await;

            assert!(server.connected(), "a responsive client was disconnected");
            assert!(!server.released(), "pending states were reset");
            assert!(
                pings.load(Ordering::Relaxed) >= 3,
                "the server did not ping the client"
            );
            reader.abort();
            server.stop().await;
        });
    }

    #[test]
    fn disabled_heartbeat_neither_pings_nor_disconnects_a_silent_client() {
        if crate::test_support::isolated() {
            return;
        }
        block_on(async {
            let server = TestServer::start(None).await;
            let mut client = connect(server.addr).await;
            assert!(
                until(DEADLINE, || server.connected()).await,
                "the client was not accepted"
            );

            // The client sends nothing after the handshake.
            let quiet = Instant::now() + TEST_HEARTBEAT.timeout * 2;
            while let Ok(message) = timeout_at(quiet, client.next()).await {
                let Some(message) = message else {
                    panic!("the server closed the connection");
                };
                assert!(
                    !matches!(message, Ok(Message::Ping(_))),
                    "a disabled heartbeat pinged the client"
                );
            }

            assert!(server.connected(), "a silent client was disconnected");
            assert!(!server.released(), "pending states were reset");
            server.stop().await;
        });
    }

    #[test]
    fn heartbeat_ends_a_send_to_a_client_that_stops_reading() {
        if crate::test_support::isolated() {
            return;
        }
        block_on(async {
            let server = TestServer::start(Some(TEST_HEARTBEAT)).await;
            // A small receive buffer fills the socket buffers sooner, so the
            // server's send waits for space the client never frees.
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.set_recv_buffer_size(4096).unwrap();
            let stream = socket.connect(server.addr).await.unwrap();
            let (mut client, _) =
                tokio_tungstenite::client_async(format!("ws://{}", server.addr), stream)
                    .await
                    .unwrap();
            send_handshake(&mut client).await;
            assert!(
                until(DEADLINE, || server.connected()).await,
                "the client was not accepted"
            );

            for _ in 0..32 {
                server.sender.send(FastVec::Heap(vec![0; 1 << 20]));
            }

            assert!(
                until(DEADLINE, || !server.connected()).await,
                "the blocked send to the silent client did not end"
            );
            assert!(server.released(), "pending states were not released");
            server.stop().await;
        });
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;

    #[test]
    fn required_handshake_fields_reject_missing_or_wrong_credentials() {
        let settings = Handshake {
            version: Some(17),
            token: Some("secret".into()),
            origins: Default::default(),
        };
        for (protocol, version, token, diagnostic) in [
            (0, Some(17), Some("secret"), "protocol version"),
            (
                PROTOCOL_VERSION + 1,
                Some(17),
                Some("secret"),
                "protocol version",
            ),
            (PROTOCOL_VERSION, None, Some("secret"), "version missing"),
            (PROTOCOL_VERSION, Some(18), Some("secret"), "wrong version"),
            (PROTOCOL_VERSION, Some(17), None, "token missing"),
            (PROTOCOL_VERSION, Some(17), Some("wrong"), "wrong token"),
            (PROTOCOL_VERSION, Some(17), Some("secre"), "wrong token"),
            (PROTOCOL_VERSION, Some(17), Some("secrets"), "wrong token"),
            (PROTOCOL_VERSION, Some(17), Some(""), "wrong token"),
        ] {
            let error = check_handshake(&settings, protocol, version, token.map(str::to_owned))
                .unwrap_err();
            assert!(error.contains(diagnostic), "{error}");
            assert!(
                check_handshake(&settings, PROTOCOL_VERSION, Some(17), Some("secret".into()))
                    .is_ok()
            );
        }
    }
}
