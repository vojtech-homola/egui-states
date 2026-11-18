use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

use futures_util::FutureExt;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, Interval, MissedTickBehavior, interval_at, timeout_at};

use crate::PROTOCOL_VERSION;
use crate::event::Event;
use crate::file_transfer::engine::Engine;
use crate::protocol::{FRAME_FLUSH_SIZE, FastVec, MAX_MSG_COUNT, ServerHeader, serialize};
use crate::server_core::http::origin::OriginPolicy;
use crate::server_core::http::web::{WebSite, WebSlot};
use crate::server_core::net::heartbeat::{Activity, Heartbeat};
use crate::server_core::net::sender::{LaneReceiver, MessageReceiver, MessageSender, SenderData};
use crate::server_core::net::socket_reader::{ClientMessage, SocketReader};
use crate::server_core::net::transport::{
    FrameSink, FrameSource, Listener, Opened, Outgoing, Served,
};
use crate::server_core::server::ServerStatesList;
use crate::server_core::signals::SignalsManager;

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
struct ClientReady<L: Listener> {
    socket_tx: L::Sink,
    socket_reader: SocketReader<L::Source>,
    peer_addr: String,
}

/// What a new connection came to.
enum Accepted<L: Listener> {
    /// It passed the handshake.
    Client(ClientReady<L>),
    /// It claimed a browser download, whose data is still to be streamed.
    Download(Pin<Box<dyn Future<Output = ()> + Send>>),
    /// It was answered or refused.
    Nothing,
}

/// What the run loop shares with the tasks of each connection.
#[derive(Clone)]
pub(crate) struct Session {
    pub(crate) sender: MessageSender,
    pub(crate) connected: Arc<AtomicBool>,
    pub(crate) values: ServerStatesList,
    pub(crate) signals: SignalsManager,
    pub(crate) heartbeat: Option<Heartbeat>,
    /// The engine of the file transfer endpoints.
    pub(crate) transfers: Engine,
    /// Whether any state uses the low-priority lane.
    pub(crate) low_priority: bool,
}

/// The file data lane of one connection.
struct DataLane {
    /// Chunks the engine's pump has prepared.
    chunks: mpsc::Receiver<Vec<u8>>,
    /// The connection's generation in the engine.
    generation: u64,
    pump: JoinHandle<()>,
    /// Writes received files.
    sinks: JoinHandle<()>,
    /// Expires the tickets of browser downloads the browser did not fetch.
    tickets: JoinHandle<()>,
}

pub(crate) async fn run<L: Listener>(
    mut listener: L,
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
        transfers,
        low_priority,
        ..
    } = session.clone();
    let mut holder = ChannelHolder::Rx(rx);
    // Connections still handshaking or receiving the web page. They run
    // concurrently so an idle connection cannot hold up the others; dropping
    // the set on shutdown aborts them. At most `MAX_PENDING_CONNECTIONS` run at
    // once.
    let mut pending: JoinSet<Accepted<L>> = JoinSet::new();
    // Browser downloads that are streaming. A stream takes as long as the
    // browser does, so it leaves the pending connections once it has claimed
    // its transfer; each transfer has one stream at a time. Dropping the set
    // on shutdown aborts them.
    let mut downloads: JoinSet<()> = JoinSet::new();

    loop {
        // check for stop event, a finished handshake or incoming connection
        let client = tokio::select! {
            biased;
            _ = stop_event.wait_clear_async() => {
                break;
            }
            Some(result) = pending.join_next(), if !pending.is_empty() => {
                match result {
                    Ok(Accepted::Client(client)) => client,
                    Ok(Accepted::Download(streaming)) => {
                        downloads.spawn(streaming);
                        continue;
                    }
                    Ok(Accepted::Nothing) => continue,
                    Err(e) => {
                        if e.is_panic() {
                            signals.error("connection task panicked");
                        }
                        continue;
                    }
                }
            }
            Some(result) = downloads.join_next(), if !downloads.is_empty() => {
                if result.is_err_and(|e| e.is_panic()) {
                    signals.error("download task panicked");
                }
                continue;
            }
            s = listener.accept(), if pending.len() < MAX_PENDING_CONNECTIONS => {
                match s {
                    Ok(connection) => {
                        let site = web.read().clone();
                        pending.spawn(accept_client::<L>(
                            connection,
                            site,
                            handshake.clone(),
                            signals.clone(),
                            transfers.clone(),
                        ));
                    }
                    Err(e) => {
                        signals.error(e);
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
        rx.clear();

        // Transfers are attached before the connection is published, so a
        // send never finds a client without them. Their control frames share
        // the state queue and are held back until the initial state is in
        // it: the client learns of an offer only after that state.
        let held = Arc::new(Mutex::new(Some(Vec::new())));
        let (lane_sender, lane_held) = (sender.clone(), held.clone());
        let generation = transfers.attach(Arc::new(move |message| {
            let mut held = lane_held.lock();
            match held.as_mut() {
                Some(held) => held.push(message),
                None => lane_sender.send(FastVec::Heap(message)),
            }
        }));

        // The client takes the transfer limits from the first message, before
        // any transfer frame; nothing else is queued while disconnected.
        let limits = transfers.limits();
        let limits = ServerHeader::TransferLimits(
            u32::try_from(limits.window).unwrap_or(u32::MAX),
            limits.chunk as u32,
        );
        let announced = serialize(&limits).map(|data| sender.send(data)).is_ok();

        connected.store(true, Ordering::Release);
        let failure = if !announced {
            Some("failed to serialize the transfer limits")
        } else if values.sync.iter().any(|v| v.sync().is_err()) {
            Some("failed to sync value after handshake")
        } else {
            // The low-priority lane synchronizes behind the normal one, so it
            // requests its own repaint once its states have arrived.
            let repaint = || serialize(&ServerHeader::Update(0.0));
            match (repaint(), repaint()) {
                (Ok(data), Ok(low)) => {
                    sender.send(data);
                    if low_priority {
                        sender.lane(true).send(low);
                    }
                    None
                }
                _ => Some("failed to serialize update message"),
            }
        };
        if let Some(error) = failure {
            // Drop only this client; the server keeps accepting connections.
            // The partial sync may have left producers waiting for this
            // client's acknowledgements, so release them.
            signals.error(error);
            connected.store(false, Ordering::Release);
            transfers.detach(generation);
            for v in values.ack.values() {
                v.reset();
            }
            holder = ChannelHolder::Rx(rx);
            continue;
        }

        // The initial state is queued; the frames held back follow it, in
        // their order, and later ones go straight into the queue.
        {
            let mut held = held.lock();
            for message in held.take().into_iter().flatten() {
                sender.send(FastVec::Heap(message));
            }
        }
        let (chunk_tx, chunks) = mpsc::channel(1);
        let pump_engine = transfers.clone();
        let pump = tokio::spawn(async move {
            let pump = std::panic::AssertUnwindSafe(pump_engine.pump(generation, chunk_tx));
            // A panicking pump would leave the transfers stuck; fail them.
            if futures_util::FutureExt::catch_unwind(pump).await.is_err() {
                pump_engine.detach(generation);
            }
        });
        let sink_engine = transfers.clone();
        let sinks = tokio::spawn(async move { sink_engine.sink_worker(generation).await });
        let ticket_engine = transfers.clone();
        let tickets = tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(1));
            loop {
                every.tick().await;
                ticket_engine.expire_tickets();
            }
        });
        let lane = DataLane {
            chunks,
            generation,
            pump,
            sinks,
            tickets,
        };

        // After the transfers are attached, so a connect callback can send.
        signals.on_connect(peer_addr);

        let activity = socket_reader.activity();
        let answers = Arc::new(LaneAnswers::default());
        let reader_handler = tokio::spawn(reader(socket_reader, session.clone(), answers.clone()));
        let handler = tokio::spawn(writer(
            rx,
            socket_tx,
            reader_handler,
            activity,
            session.clone(),
            lane,
            answers,
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
async fn accept_client<L: Listener>(
    connection: L::Connection,
    site: Option<Arc<WebSite>>,
    handshake: Handshake,
    signals: SignalsManager,
    transfers: Engine,
) -> Accepted<L> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let connection = match L::serve_web(connection, site, &transfers, deadline, &signals).await {
        Served::Connection(connection) => connection,
        Served::Download(streaming) => return Accepted::Download(streaming),
        Served::Done => return Accepted::Nothing,
    };

    match timeout_at(
        deadline,
        handshake_client::<L>(connection, &handshake, &signals),
    )
    .await
    {
        Ok(Some(client)) => Accepted::Client(client),
        Ok(None) => Accepted::Nothing,
        Err(_) => {
            signals.warning("client handshake timed out");
            Accepted::Nothing
        }
    }
}

async fn handshake_client<L: Listener>(
    connection: L::Connection,
    handshake: &Handshake,
    signals: &SignalsManager,
) -> Option<ClientReady<L>> {
    let Opened {
        sink: socket_tx,
        source: socket_rx,
        peer_addr,
    } = L::open(connection, &handshake.origins, signals).await?;
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

/// Reads the client's frames. Answers to the marks of the low-priority lane go
/// to the writer through `answers`.
async fn reader<S: FrameSource>(
    mut socket_rx: SocketReader<S>,
    session: Session,
    answers: Arc<LaneAnswers>,
) {
    let Session {
        sender,
        connected,
        values,
        signals,
        transfers,
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
            // The engine never waits here: sinks that block have a worker.
            Ok(ClientMessage::Transfer(endpoint, frame, payload)) => {
                transfers.handle_frame(endpoint, frame, payload);
            }
            Ok(ClientMessage::LaneAck) => answers.answer(),
        }
    }

    // send close signal to writing thread if reading fails
    #[cfg(debug_assertions)]
    signals.debug("terminating write thread");
    sender.close();
}

/// After this many frames of the normal lane in a row, the writer sends a
/// waiting frame of the low-priority lane or a file chunk first, taking turns
/// between the two. A busy normal lane then slows the other lanes down but
/// never stops them.
const STARVATION_GUARD: usize = 8;

/// The client's answers to the marks of the low-priority lane, counted by the
/// reader for the writer.
///
/// A counter instead of a queue: a client that sends answers without end
/// costs the server no memory, and the writer takes all of them in one step.
#[derive(Default)]
struct LaneAnswers {
    count: AtomicUsize,
    arrived: Notify,
}

impl LaneAnswers {
    fn answer(&self) {
        self.count.fetch_add(1, Ordering::AcqRel);
        self.arrived.notify_one();
    }

    /// The answers that arrived since the last call.
    fn take(&self) -> usize {
        self.count.swap(0, Ordering::AcqRel)
    }
}

/// The low-priority lane of one connection.
///
/// The client answers the mark that follows every frame once it has handled
/// that frame, so the bytes of unanswered frames are still on the way. The
/// lane sends while they are below the window, so it never fills the socket
/// buffers with bulk data that a normal message would wait behind.
struct LowLane {
    receiver: DataReceiver,
    open: bool,
    window: usize,
    /// The bytes of every frame whose mark is unanswered, oldest first.
    unanswered: VecDeque<usize>,
    in_flight: usize,
}

impl LowLane {
    fn new(receiver: LaneReceiver, window: usize) -> Self {
        Self {
            receiver: DataReceiver::new(receiver),
            open: true,
            window,
            unanswered: VecDeque::new(),
            in_flight: 0,
        }
    }

    fn has_room(&self) -> bool {
        self.open && self.in_flight < self.window
    }

    fn sent(&mut self, len: usize) {
        self.in_flight += len;
        self.unanswered.push_back(len);
    }

    /// Returns the bytes of the `count` oldest unanswered frames to the
    /// window. Answers beyond the marks that were sent are ignored.
    fn answered(&mut self, count: usize) {
        let count = count.min(self.unanswered.len());
        for len in self.unanswered.drain(..count) {
            self.in_flight -= len;
        }
    }
}

enum Next {
    Message(Option<SenderData>),
    Low(Option<SenderData>),
    Chunk(Vec<u8>),
    Ping,
}

/// A frame of the low-priority lane or a file chunk that is ready now; the
/// chunk first with `chunk_first`.
fn lagging(
    low: &mut LowLane,
    chunks: &mut mpsc::Receiver<Vec<u8>>,
    chunks_open: bool,
    chunk_first: bool,
) -> Option<Next> {
    let chunk = |chunks: &mut mpsc::Receiver<Vec<u8>>| match chunks_open {
        true => chunks.try_recv().ok().map(Next::Chunk),
        false => None,
    };
    if chunk_first && let Some(chunk) = chunk(chunks) {
        return Some(chunk);
    }
    if low.has_room()
        && let Some(message) = low.receiver.next().now_or_never()
    {
        return Some(Next::Low(message));
    }
    match chunk_first {
        true => None,
        false => chunk(chunks),
    }
}

#[allow(clippy::too_many_arguments)]
async fn writer<K: FrameSink>(
    rx: MessageReceiver,
    mut websocket: K,
    reader_handle: tokio::task::JoinHandle<()>,
    activity: Arc<Activity>,
    session: Session,
    lane: DataLane,
    answers: Arc<LaneAnswers>,
) -> MessageReceiver {
    let Session {
        connected,
        values,
        signals,
        heartbeat,
        transfers,
        ..
    } = session;
    let DataLane {
        mut chunks,
        generation,
        pump,
        sinks,
        tickets,
    } = lane;
    let mut chunks_open = true;

    let MessageReceiver { normal, low } = rx;
    let mut data_receiver = DataReceiver::new(normal);
    let window = usize::try_from(transfers.limits().window).unwrap_or(usize::MAX);
    let mut low = LowLane::new(low, window);
    let lane_mark = serialize::<_, 32>(&ServerHeader::LaneMark)
        .map(FastVec::into_bytes)
        .unwrap_or_default();
    let mut normal_streak = 0;
    // Whether the starvation guard's next turn belongs to the file lane.
    let mut chunk_turn = false;

    // The connection counts as heard from when it starts, however long the
    // initial synchronization took.
    activity.touch();
    let mut pings = heartbeat.map(|heartbeat| {
        let mut pings = interval_at(Instant::now() + heartbeat.interval, heartbeat.interval);
        pings.set_missed_tick_behavior(MissedTickBehavior::Delay);
        pings
    });

    loop {
        // Taken on every turn, so answers never wait behind the lanes, and
        // a flood of them costs one step.
        low.answered(answers.take());
        let due = if normal_streak >= STARVATION_GUARD {
            normal_streak = 0;
            let due = lagging(&mut low, &mut chunks, chunks_open, chunk_turn);
            match due {
                Some(Next::Low(_)) => chunk_turn = true,
                Some(Next::Chunk(_)) => chunk_turn = false,
                _ => {}
            }
            due
        } else {
            None
        };
        // Pings go first, so a long burst of state messages cannot starve
        // the heartbeat. The low-priority lane and then file data go only
        // when no normal message waits, apart from the starvation guard. An
        // answer only wakes an idle writer, so answers cannot hold up a lane.
        // Every branch is cancel safe: each only awaits a channel, a
        // notification or a timer tick, and a missed tick is delivered by
        // the next poll.
        let next = match due {
            Some(next) => next,
            None => tokio::select! {
                biased;
                () = next_ping(&mut pings) => Next::Ping,
                message = data_receiver.next() => Next::Message(message),
                message = low.receiver.next(), if low.has_room() => Next::Low(message),
                chunk = chunks.recv(), if chunks_open => match chunk {
                    Some(chunk) => Next::Chunk(chunk),
                    None => {
                        chunks_open = false;
                        continue;
                    }
                },
                () = answers.arrived.notified() => continue,
            },
        };

        let (message, mark) = match next {
            Next::Message(Some(message)) => {
                // if not connected, stop thread
                if !connected.load(Ordering::Acquire) {
                    unless_silent(websocket.close(), heartbeat, &activity).await;
                    break;
                }
                normal_streak += 1;
                (Outgoing::Binary(message.into_bytes()), false)
            }
            Next::Low(Some(message)) => {
                if !connected.load(Ordering::Acquire) {
                    unless_silent(websocket.close(), heartbeat, &activity).await;
                    break;
                }
                normal_streak = 0;
                low.sent(message.len());
                (Outgoing::Binary(message.into_bytes()), true)
            }
            Next::Low(None) => {
                low.open = false;
                continue;
            }
            Next::Chunk(chunk) => {
                if !connected.load(Ordering::Acquire) {
                    unless_silent(websocket.close(), heartbeat, &activity).await;
                    break;
                }
                normal_streak = 0;
                (Outgoing::Binary(chunk.into()), false)
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
                (Outgoing::Ping, false)
            }
        };

        if !send(&mut websocket, message, heartbeat, &activity, &signals).await {
            break;
        }
        if mark
            && !send(
                &mut websocket,
                Outgoing::Binary(lane_mark.clone()),
                heartbeat,
                &activity,
                &signals,
            )
            .await
        {
            break;
        }
    }

    reader_handle.abort();
    let _ = reader_handle.await;
    // Every unfinished transfer fails as disconnected.
    transfers.detach(generation);
    pump.abort();
    sinks.abort();
    tickets.abort();
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

    MessageReceiver {
        normal: data_receiver.finalize(),
        low: low.receiver.finalize(),
    }
}

/// Sends `frame`, and reports whether the connection still works.
///
/// A dead client stops reading, so a send can wait for buffer space forever;
/// the heartbeat also ends such a send.
async fn send<K: FrameSink>(
    websocket: &mut K,
    frame: Outgoing,
    heartbeat: Option<Heartbeat>,
    activity: &Activity,
    signals: &SignalsManager,
) -> bool {
    match unless_silent(websocket.send_frame(frame), heartbeat, activity).await {
        Some(Ok(())) => true,
        Some(Err(e)) => {
            signals.error(format!("sending message to client failed: {}", e));
            false
        }
        None => {
            if let Some(heartbeat) = heartbeat {
                report_silent(signals, heartbeat);
            }
            false
        }
    }
}

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
/// appending would copy them again, and they fill a frame on their own. In
/// the `burst` benchmark merging 16 KiB messages is a gain and merging
/// 100 KiB messages a loss.
const MERGE_LIMIT: usize = 64 * 1024;

struct DataReceiver {
    rx: LaneReceiver,
    send_next: Option<(SenderData, bool)>,
    is_closed: bool,
}

impl DataReceiver {
    fn new(rx: LaneReceiver) -> Self {
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
        while counter < MAX_MSG_COUNT && msg.len() < FRAME_FLUSH_SIZE {
            match self.rx.try_recv() {
                Ok(Some((next_msg, send_now))) => {
                    if send_now || next_msg.len() > MERGE_LIMIT {
                        self.send_next = Some((next_msg, send_now));
                        break;
                    }
                    msg.extend_from_data(&next_msg);
                    counter += 1;
                }
                Err(TryRecvError::Empty) => break,
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.is_closed = true;
                    break;
                }
            }
        }
        Some(msg)
    }

    fn finalize(self) -> LaneReceiver {
        self.rx
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::{DataReceiver, MERGE_LIMIT};
    use crate::protocol::{FRAME_FLUSH_SIZE, FastVec, MAX_MSG_COUNT};

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

    /// A frame is sent once it has reached the flush size; what follows
    /// starts the next frame.
    #[tokio::test]
    async fn data_receiver_sends_a_frame_that_reached_the_flush_size() {
        let count = FRAME_FLUSH_SIZE / MERGE_LIMIT;
        let mut messages: Vec<_> = (0..count).map(|_| (data(MERGE_LIMIT), false)).collect();
        messages.push((data(11), false));
        let frames = frames(messages).await;
        assert_eq!(frames, [joined(&vec![MERGE_LIMIT; count]), bytes(11)]);
    }

    /// One message short of the flush size, the frame still takes the next.
    #[tokio::test]
    async fn data_receiver_merges_below_the_flush_size() {
        let count = FRAME_FLUSH_SIZE / MERGE_LIMIT;
        let mut sizes = vec![MERGE_LIMIT; count - 1];
        sizes.extend([MERGE_LIMIT - 1, 11]);
        let messages = sizes.iter().map(|size| (data(*size), false)).collect();
        let frames = frames(messages).await;
        assert_eq!(frames, [joined(&sizes)]);
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
        let marker = |index: usize| (index % 251) as u8;
        let (tx, rx) = unbounded_channel();
        for index in 0..MAX_MSG_COUNT + 2 {
            tx.send(Some((FastVec::Heap(vec![marker(index)]), false)))
                .unwrap();
        }
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            (0..MAX_MSG_COUNT).map(marker).collect::<Vec<_>>()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            [marker(MAX_MSG_COUNT), marker(MAX_MSG_COUNT + 1)]
        );
        assert!(receiver.next().await.is_none());
    }
}

#[cfg(test)]
mod run_tests {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::atomic::AtomicUsize;

    use bytes::Bytes;
    use futures_util::SinkExt;
    use parking_lot::RwLock;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;
    use crate::file_transfer::engine::Role;
    use crate::file_transfer::shared::TransferShared;
    use crate::file_transfer::source::Source;
    use crate::file_transfer::{TransferError, TransferProfile};
    use crate::protocol::file_transfer::{FileEntry, OfferBody};
    use crate::protocol::{ClientHeader, FastVec};
    use crate::server_core::net::transport::Incoming;
    use crate::server_core::net::transport::memory::{self, MemoryClient, MemoryConnector};
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

    fn handshake() -> Vec<u8> {
        postcard::to_stdvec(&ClientHeader::Handshake(PROTOCOL_VERSION, None, None)).unwrap()
    }

    /// Opens an in-memory connection and sends the protocol handshake.
    fn connect(connector: &MemoryConnector) -> MemoryClient {
        let client = connector.connect().unwrap();
        client
            .send
            .send(Incoming::Binary(handshake().into()))
            .unwrap();
        client
    }

    const TEST_HEARTBEAT: Heartbeat = Heartbeat {
        interval: Duration::from_millis(100),
        timeout: Duration::from_millis(400),
    };
    const DEADLINE: Duration = Duration::from_secs(5);
    /// The endpoint the tests send transfers on.
    const ENDPOINT: u64 = 1000;

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
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        stop_event: Event,
        probe: Arc<ResetProbe>,
        transfers: Engine,
        handle: JoinHandle<MessageReceiver>,
    }

    impl TestServer {
        /// Runs the loop on in-memory connections.
        fn memory(heartbeat: Option<Heartbeat>) -> (Self, MemoryConnector) {
            Self::memory_with(heartbeat, SignalsManager::new(), Vec::new())
        }

        fn memory_with(
            heartbeat: Option<Heartbeat>,
            signals: SignalsManager,
            sync: Vec<Arc<dyn SyncTrait>>,
        ) -> (Self, MemoryConnector) {
            let (listener, connector) = memory::listener();
            (Self::run(listener, heartbeat, signals, sync), connector)
        }

        /// Runs the loop on WebSockets over TCP.
        async fn tcp(heartbeat: Option<Heartbeat>) -> (Self, SocketAddr) {
            let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let addr = listener.local_addr().unwrap();
            let listener = crate::server_core::net::transport::SocketListener::new(listener);
            let server = Self::run(listener, heartbeat, SignalsManager::new(), Vec::new());
            (server, addr)
        }

        fn run<L: Listener>(
            listener: L,
            heartbeat: Option<Heartbeat>,
            signals: SignalsManager,
            sync: Vec<Arc<dyn SyncTrait>>,
        ) -> Self {
            let (sender, rx) = MessageSender::new();
            let connected = Arc::new(AtomicBool::new(false));
            let stop_event = Event::new();
            let probe = Arc::new(ResetProbe(AtomicBool::new(false)));
            let mut values = ServerStatesList {
                sync,
                ..Default::default()
            };
            values.ack.insert(1, probe.clone());
            let transfers = Engine::new(crate::file_transfer::engine::Side::Server, None);
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
                    transfers: transfers.clone(),
                    low_priority: false,
                },
            ));
            Self {
                sender,
                connected,
                stop_event,
                probe,
                transfers,
                handle,
            }
        }

        fn connected(&self) -> bool {
            self.connected.load(Ordering::Acquire)
        }

        /// Offers one byte on [`ENDPOINT`].
        fn send_transfer(&self) -> Result<Arc<TransferShared>, TransferError> {
            let body = OfferBody {
                name: "a".to_string(),
                files: vec![FileEntry {
                    name: "a".to_string(),
                    size: 1,
                    modified: None,
                }],
            };
            self.transfers
                .send(ENDPOINT, body, Bytes::new(), Source::bytes(vec![1]))
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

    /// Records the frames a writer sends.
    struct FrameLog(mpsc::UnboundedSender<Outgoing>);

    impl FrameSink for FrameLog {
        async fn send_frame(&mut self, frame: Outgoing) -> Result<(), String> {
            self.0
                .send(frame)
                .map_err(|_| "the log is gone".to_string())
        }

        async fn close(&mut self) {}
    }

    /// A writer started on messages queued before it starts, so the lane it
    /// picks first is deterministic.
    struct WriterRun {
        frames: mpsc::UnboundedReceiver<Outgoing>,
        answers: Arc<LaneAnswers>,
        sender: MessageSender,
        handle: JoinHandle<MessageReceiver>,
    }

    impl WriterRun {
        /// Starts the writer with a low-priority window of `window` bytes and
        /// `chunk` waiting in the file lane.
        fn start(
            sender: MessageSender,
            rx: MessageReceiver,
            window: u64,
            chunk: Option<Vec<u8>>,
        ) -> Self {
            let transfers = Engine::new(crate::file_transfer::engine::Side::Server, None);
            transfers.set_limits(crate::file_transfer::TransferLimits::new(window, 1).unwrap());
            let (log, frames) = mpsc::unbounded_channel();
            let (chunk_tx, chunks) = mpsc::channel(1);
            if let Some(chunk) = chunk {
                chunk_tx.try_send(chunk).unwrap();
            }
            let idle = || tokio::spawn(std::future::pending::<()>());
            let lane = DataLane {
                chunks,
                generation: 0,
                pump: idle(),
                sinks: idle(),
                tickets: idle(),
            };
            let session = Session {
                sender: sender.clone(),
                connected: Arc::new(AtomicBool::new(true)),
                values: Default::default(),
                signals: SignalsManager::new(),
                heartbeat: None,
                transfers,
                low_priority: true,
            };
            let answers = Arc::new(LaneAnswers::default());
            let writer_answers = answers.clone();
            let handle = tokio::spawn(async move {
                // Keeps the file lane open for the run.
                let _chunk_tx = chunk_tx;
                writer(
                    rx,
                    FrameLog(log),
                    idle(),
                    Activity::new(),
                    session,
                    lane,
                    writer_answers,
                )
                .await
            });
            Self {
                frames,
                answers,
                sender,
                handle,
            }
        }

        /// The first byte and the length of the next frame; the mark of the
        /// low-priority lane is `LANE_MARK`.
        async fn next(&mut self) -> (u8, usize) {
            match tokio::time::timeout(DEADLINE, self.frames.recv()).await {
                Ok(Some(Outgoing::Binary(frame))) => (frame[0], frame.len()),
                other => panic!("expected a frame, got {:?}", other.ok().flatten().is_some()),
            }
        }

        async fn assert_quiet(&mut self) {
            assert!(
                tokio::time::timeout(Duration::from_millis(100), self.frames.recv())
                    .await
                    .is_err(),
                "the writer sent a frame"
            );
        }

        async fn stop(self) {
            self.sender.close();
            tokio::time::timeout(DEADLINE, self.handle)
                .await
                .expect("the writer did not stop")
                .unwrap();
        }
    }

    /// The wire tag of `ServerHeader::LaneMark`.
    const LANE_MARK: u8 = 14;
    /// Larger than the merge limit, so every message is a frame of its own.
    const LARGE: usize = MERGE_LIMIT + 1;

    fn message(marker: u8, len: usize) -> SenderData {
        FastVec::Heap(vec![marker; len])
    }

    /// Normal messages go first. The low-priority lane then sends while its
    /// unanswered bytes are below the window, each frame followed by a mark.
    #[test]
    fn the_low_lane_waits_for_the_normal_lane_and_for_its_window() {
        block_on(async {
            let (sender, rx) = MessageSender::new();
            let low = sender.lane(true);
            low.send(message(1, LARGE));
            low.send(message(2, LARGE));
            sender.send(message(9, 10));
            let mut run = WriterRun::start(sender, rx, 1024, None);

            assert_eq!(run.next().await, (9, 10));
            assert_eq!(run.next().await, (1, LARGE));
            assert_eq!(run.next().await, (LANE_MARK, 1));
            // A frame larger than the window fills it.
            run.assert_quiet().await;
            run.answers.answer();
            assert_eq!(run.next().await, (2, LARGE));
            assert_eq!(run.next().await, (LANE_MARK, 1));
            run.stop().await;
        });
    }

    /// Answers are counted, and an answer without an unanswered mark returns
    /// nothing to the window.
    #[test]
    fn surplus_lane_answers_are_ignored() {
        let answers = LaneAnswers::default();
        for _ in 0..3 {
            answers.answer();
        }
        assert_eq!(answers.take(), 3);
        assert_eq!(answers.take(), 0);

        let (_sender, rx) = MessageSender::new();
        let mut low = LowLane::new(rx.low, 100);
        low.answered(5);
        low.sent(60);
        low.sent(60);
        assert!(!low.has_room());
        low.answered(1);
        assert_eq!(low.in_flight, 60);
        assert!(low.has_room());
        low.answered(9);
        assert_eq!(low.in_flight, 0);
    }

    /// A normal lane that never empties holds the other lanes back for at
    /// most `STARVATION_GUARD` frames at a time.
    #[test]
    fn the_starvation_guard_lets_the_other_lanes_through() {
        block_on(async {
            for low_waits in [true, false] {
                let (sender, rx) = MessageSender::new();
                let count = 3 * STARVATION_GUARD;
                for marker in 0..count {
                    sender.send(message(100 + marker as u8, LARGE));
                }
                if low_waits {
                    sender.lane(true).send(message(1, LARGE));
                }
                let mut run = WriterRun::start(sender, rx, 1 << 20, Some(vec![2; 100]));

                let mut sent = 0..count;
                let mut normal = async |run: &mut WriterRun| {
                    for marker in sent.by_ref().take(STARVATION_GUARD) {
                        assert_eq!(run.next().await, (100 + marker as u8, LARGE));
                    }
                };
                normal(&mut run).await;
                // The two other lanes take turns, the low-priority lane first.
                if low_waits {
                    assert_eq!(run.next().await, (1, LARGE));
                    assert_eq!(run.next().await, (LANE_MARK, 1));
                    normal(&mut run).await;
                }
                assert_eq!(run.next().await, (2, 100));
                normal(&mut run).await;
                run.stop().await;
            }
        });
    }

    #[test]
    fn failed_sync_drops_only_that_client() {
        block_on(async {
            let sync: Arc<dyn SyncTrait> = Arc::new(FailFirstSync(AtomicUsize::new(0)));
            let (server, connector) =
                TestServer::memory_with(None, SignalsManager::new(), vec![sync]);

            server.transfers.register(ENDPOINT, Role::Send, 0);
            let mut rejected = connect(&connector);
            let closed = tokio::time::timeout(DEADLINE, rejected.frames.recv())
                .await
                .expect("the client whose sync failed was not dropped");
            assert!(closed.is_none(), "the rejected client received a frame");
            assert!(server.released(), "pending states were not reset");
            assert!(!server.connected());
            // Its transfers are detached with it.
            assert_eq!(
                server.send_transfer().err(),
                Some(TransferError::NotConnected)
            );

            let mut accepted = connect(&connector);
            let update = tokio::time::timeout(DEADLINE, accepted.frames.recv())
                .await
                .expect("the next client was not synchronized");
            assert!(matches!(update, Some(Outgoing::Binary(_))));
            assert!(server.connected());
            server.stop().await;
        });
    }

    /// Stays in the sync of a connection until it is released.
    struct HeldSync {
        entered: AtomicBool,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl SyncTrait for HeldSync {
        fn sync(&self) -> Result<(), ()> {
            self.entered.store(true, Ordering::Release);
            self.release.lock().recv_timeout(DEADLINE).map_err(|_| ())
        }
    }

    #[test]
    fn a_transfer_sent_during_the_initial_sync_is_offered_after_it() {
        use crate::protocol::file_transfer::TransferFrame;

        block_on(async {
            let (release, released) = std::sync::mpsc::channel();
            let sync = Arc::new(HeldSync {
                entered: AtomicBool::new(false),
                release: Mutex::new(released),
            });
            let (server, connector) =
                TestServer::memory_with(None, SignalsManager::new(), vec![sync.clone()]);
            server.transfers.register(ENDPOINT, Role::Send, 0);
            let limits = TransferProfile::Responsive.limits();
            server.transfers.set_limits(limits);
            let mut client = connect(&connector);
            assert!(
                until(DEADLINE, || sync.entered.load(Ordering::Acquire)).await,
                "the client was not accepted"
            );

            // The connection is published while its state is still queued;
            // a transfer can be sent from then on.
            assert!(server.connected());
            let sent = server.send_transfer();
            assert!(sent.is_ok(), "{:?}", sent.err());
            release.send(()).unwrap();

            // The client gets the transfer limits first and the offer after
            // the state.
            let mut received = Vec::new();
            let mut headers = Vec::new();
            while headers.len() < 3 {
                let frame = tokio::time::timeout(DEADLINE, client.frames.recv())
                    .await
                    .expect("the client was not synchronized");
                let Some(Outgoing::Binary(frame)) = frame else {
                    panic!("the connection ended");
                };
                received.extend_from_slice(&frame);
                let mut rest = &received[..];
                headers.clear();
                while let Ok((header, after)) = postcard::take_from_bytes::<ServerHeader>(rest) {
                    rest = match header {
                        ServerHeader::Transfer(_, _, len) => &after[len as usize..],
                        _ => after,
                    };
                    headers.push(header);
                }
            }
            assert!(matches!(
                headers[0],
                ServerHeader::TransferLimits(window, chunk)
                    if u64::from(window) == limits.window && chunk as usize == limits.chunk
            ));
            assert!(matches!(headers[1], ServerHeader::Update(_)));
            assert!(matches!(
                headers[2],
                ServerHeader::Transfer(ENDPOINT, TransferFrame::Offer { .. }, _)
            ));
            server.stop().await;
        });
    }

    #[test]
    fn heartbeat_disconnects_a_client_that_stops_reading() {
        block_on(async {
            let (server, connector) = TestServer::memory(Some(TEST_HEARTBEAT));
            // The client never answers a ping.
            let _client = connect(&connector);
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
        block_on(async {
            const STATE_ID: u64 = 100;
            let signals = SignalsManager::new();
            signals.set_register(STATE_ID, true, false);
            let (server, connector) =
                TestServer::memory_with(Some(TEST_HEARTBEAT), signals.clone(), Vec::new());
            let _client = connect(&connector);
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
        block_on(async {
            let (server, connector) = TestServer::memory(Some(TEST_HEARTBEAT));
            let mut client = connect(&connector);
            assert!(
                until(DEADLINE, || server.connected()).await,
                "the client was not accepted"
            );

            // The client answers every ping, as a WebSocket peer does, and
            // sends nothing else.
            let pings = Arc::new(AtomicUsize::new(0));
            let reader = tokio::spawn({
                let pings = pings.clone();
                async move {
                    while let Some(frame) = client.frames.recv().await {
                        if matches!(frame, Outgoing::Ping) {
                            pings.fetch_add(1, Ordering::Relaxed);
                            let _ = client.send.send(Incoming::KeepAlive);
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
        block_on(async {
            let (server, connector) = TestServer::memory(None);
            let mut client = connect(&connector);
            assert!(
                until(DEADLINE, || server.connected()).await,
                "the client was not accepted"
            );

            // The client sends nothing after the handshake.
            let quiet = Instant::now() + TEST_HEARTBEAT.timeout * 2;
            while let Ok(frame) = timeout_at(quiet, client.frames.recv()).await {
                let Some(frame) = frame else {
                    panic!("the server closed the connection");
                };
                assert!(
                    !matches!(frame, Outgoing::Ping),
                    "a disabled heartbeat pinged the client"
                );
            }

            assert!(server.connected(), "a silent client was disconnected");
            assert!(!server.released(), "pending states were reset");
            server.stop().await;
        });
    }

    /// Needs a real socket, whose send waits for buffer space.
    #[test]
    fn heartbeat_ends_a_send_to_a_client_that_stops_reading() {
        if crate::test_support::isolated() {
            return;
        }
        block_on(async {
            let (server, addr) = TestServer::tcp(Some(TEST_HEARTBEAT)).await;
            // A small receive buffer fills the socket buffers sooner, so the
            // server's send waits for space the client never frees.
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.set_recv_buffer_size(4096).unwrap();
            let stream = socket.connect(addr).await.unwrap();
            let (mut client, _) = tokio_tungstenite::client_async(format!("ws://{addr}"), stream)
                .await
                .unwrap();
            client
                .send(Message::Binary(handshake().into()))
                .await
                .unwrap();
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
            drop(client);
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
