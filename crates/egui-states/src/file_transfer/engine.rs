//! The transfer engine of one peer.
//!
//! The engine keeps every endpoint of the peer and the one unfinished
//! transfer each may have. Handles, offers and the connection's reader call
//! into it from any thread: every call takes the lock, updates the state,
//! sends control frames into the state lane and wakes the pump. Control frames
//! are sent while the lock is held, so their order on the wire matches the
//! order of the changes.
//!
//! Two pieces run inside the connection instead: the pump, which reads the
//! data of the head transfer and pushes chunks into the data lane, and, on
//! native peers, the writer of file sinks. Both are tied to the generation of
//! the connection that started them, so a reconnect never mixes transfers.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use bytes::Bytes;
use parking_lot::{Condvar, Mutex};
use tokio::sync::{Notify, mpsc};

use crate::file_transfer::shared::TransferShared;
use crate::file_transfer::sink::Sink;
use crate::file_transfer::source::Source;
use crate::file_transfer::{
    FILE_TRANSFER_CHUNK, FILE_TRANSFER_WINDOW, TransferError, TransferStatus, names,
};
use crate::protocol::VALUE_MAX_SIZE;
use crate::protocol::file_transfer::{
    AcceptMode, OfferBody, TransferFrame, decode_offer, encode_client, encode_offer, encode_server,
};

/// Sends one encoded message into the state lane of the connection.
pub(crate) type StateLane = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

/// Asks the egui client to repaint, after the delay if one is given.
pub(crate) type Repaint = Arc<dyn Fn(Option<Duration>) + Send + Sync>;

/// Receives the engine's events, outside the engine's lock.
pub(crate) type EventHook = Arc<dyn Fn(EngineEvent) + Send + Sync>;

/// Starts a browser download, outside the engine's lock.
pub(crate) type TicketHook = Arc<dyn Fn(BrowserDownload) + Send + Sync>;

/// A file the browser should fetch from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BrowserDownload {
    /// The ticket, as the 32 hex digits of the download path.
    pub(crate) ticket: String,
    /// The name the browser saves the file under.
    pub(crate) name: String,
}

/// The path of a browser download below the server's root.
#[cfg_attr(
    not(any(feature = "server", feature = "python", target_arch = "wasm32")),
    allow(dead_code)
)]
pub(crate) const DOWNLOAD_PATH: &str = "/egui-states/transfer/";

/// Which peer the engine runs on; it decides how frames are encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    Server,
    Client,
}

/// What this peer does on an endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Send,
    Receive,
}

pub(crate) enum EngineEvent {
    /// An offer waits on a receiving endpoint.
    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    Offer { endpoint: u64 },
    /// A transfer the application holds a handle for has finished.
    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    Finished {
        endpoint: u64,
        handle: Arc<TransferShared>,
    },
}

pub(crate) struct Inner {
    side: Side,
    state: Mutex<State>,
    /// Signalled when an endpoint becomes free or the connection ends.
    free: Condvar,
    /// Wakes the pump.
    pump: Notify,
    repaint: Option<Repaint>,
    events: OnceLock<EventHook>,
    tickets: OnceLock<TicketHook>,
    this: Weak<Inner>,
}

/// The transfer engine of one peer; cheap to clone.
#[derive(Clone)]
pub(crate) struct Engine(Arc<Inner>);

struct State {
    connection: Option<Connection>,
    generation: u64,
    endpoints: HashMap<u64, Endpoint>,
    /// Accepted outgoing transfers in the order they were accepted. Only the
    /// head sends data.
    queue: VecDeque<(u64, u32)>,
    next_key: u64,
    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    next_waiter: u64,
    /// The job queue of the attached connection, until its worker takes it.
    #[cfg(not(target_arch = "wasm32"))]
    jobs: Option<(u64, mpsc::UnboundedReceiver<SinkJob>)>,
}

struct Connection {
    lane: StateLane,
    generation: u64,
    /// Chunks for the sinks that block, written by [`Engine::sink_worker`].
    #[cfg(not(target_arch = "wasm32"))]
    jobs: mpsc::UnboundedSender<SinkJob>,
}

/// A chunk for a sink that blocks.
#[cfg(not(target_arch = "wasm32"))]
struct SinkJob {
    endpoint: u64,
    serial: u32,
    data: Bytes,
}

/// Bytes the sink worker writes in one blocking call.
#[cfg(not(target_arch = "wasm32"))]
const SINK_BATCH: usize = 1024 * 1024;

struct Endpoint {
    role: Role,
    meta_type: u32,
    /// How many receivers take the offers; without one, offers are rejected
    /// at once.
    receivers: usize,
    next_serial: u32,
    current: Option<Transfer>,
    /// A send whose files are still being picked.
    reserved: Option<Arc<TransferShared>>,
    last: Option<Arc<TransferShared>>,
    /// Threads waiting for the endpoint to become free, in call order.
    waiters: VecDeque<u64>,
    /// Accepts every offer and drops the data, for benchmarks.
    #[cfg(any(test, feature = "bench"))]
    discard: bool,
}

impl Endpoint {
    fn is_busy(&self) -> bool {
        self.current.is_some() || self.reserved.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Offered, not answered yet.
    Offered,
    /// Sender: accepted, in the queue.
    Sending,
    /// Sender: the last chunk is out, waiting for the receiver's `Finished`.
    AwaitFinish,
    /// Receiver: accepted, taking chunks.
    Receiving,
    /// Sender: accepted as browser downloads over HTTP.
    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    Http,
}

struct Transfer {
    serial: u32,
    shared: Arc<TransferShared>,
    phase: Phase,
    body: OfferBody,
    meta: Bytes,
    total: u64,
    /// Whether the application holds a handle; only then a finished event
    /// is reported.
    exposed: bool,
    /// Receiver: whether the offer was handed to the application.
    taken: bool,
    // Sender.
    /// `None` while the pump reads from it.
    source: Option<Source>,
    credit: u64,
    sent: u64,
    // Receiver.
    sink: Option<Sink>,
    received: u64,
    consumed: u64,
    granted: u64,
    /// Sender, browser downloads: the tickets and the next file.
    #[cfg(any(feature = "server", feature = "python"))]
    http: Option<http::HttpState>,
}

impl Transfer {
    fn new(serial: u32, shared: Arc<TransferShared>, body: OfferBody, meta: Bytes) -> Self {
        let total = body.files.iter().map(|file| file.size).sum();
        Self {
            serial,
            shared,
            phase: Phase::Offered,
            body,
            meta,
            total,
            exposed: false,
            taken: false,
            source: None,
            credit: 0,
            sent: 0,
            sink: None,
            received: 0,
            consumed: 0,
            granted: 0,
            #[cfg(any(feature = "server", feature = "python"))]
            http: None,
        }
    }

    /// Whether the receiver hands the data to the browser.
    fn is_browser(&self) -> bool {
        matches!(self.sink, Some(Sink::Browser))
    }
}

/// Changes to make once the lock is released.
#[derive(Default)]
struct Effects {
    events: Vec<EngineEvent>,
    downloads: Vec<BrowserDownload>,
    wake_pump: bool,
    wake_free: bool,
}

/// An offer handed to the application.
pub(crate) struct RawOffer {
    pub(crate) token: OfferToken,
    pub(crate) body: OfferBody,
    pub(crate) meta: Bytes,
}

/// The right to answer one offer. Dropping it unanswered rejects the offer.
pub(crate) struct OfferToken {
    engine: Engine,
    endpoint: u64,
    serial: u32,
    shared: Arc<TransferShared>,
    answered: bool,
}

const NOT_ACCEPTED: &str = "the offer was not accepted";

impl Engine {
    pub(crate) fn new(side: Side, repaint: Option<Repaint>) -> Self {
        Self(Arc::new_cyclic(|this| Inner {
            side,
            state: Mutex::new(State {
                connection: None,
                generation: 0,
                endpoints: HashMap::new(),
                queue: VecDeque::new(),
                next_key: 0,
                next_waiter: 0,
                #[cfg(not(target_arch = "wasm32"))]
                jobs: None,
            }),
            free: Condvar::new(),
            pump: Notify::new(),
            repaint,
            events: OnceLock::new(),
            tickets: OnceLock::new(),
            this: this.clone(),
        }))
    }

    pub(super) fn from_inner(inner: Arc<Inner>) -> Self {
        Self(inner)
    }

    /// Sets how browser downloads start; only the first call has an effect.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn set_ticket_hook(&self, hook: TicketHook) {
        let _ = self.0.tickets.set(hook);
    }

    /// Sets where events go; only the first call has an effect.
    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    pub(crate) fn set_event_hook(&self, hook: EventHook) {
        let _ = self.0.events.set(hook);
    }

    /// Adds an endpoint. A client takes the offers of its receiving
    /// endpoints itself; a server needs a receiver set first.
    pub(crate) fn register(&self, endpoint: u64, role: Role, meta_type: u32) {
        // A client takes its offers with `take_offer` at any time.
        let receivers = usize::from(self.0.side == Side::Client);
        self.0.state.lock().endpoints.insert(
            endpoint,
            Endpoint {
                role,
                meta_type,
                receivers,
                next_serial: 0,
                current: None,
                reserved: None,
                last: None,
                waiters: VecDeque::new(),
                #[cfg(any(test, feature = "bench"))]
                discard: false,
            },
        );
    }

    /// Accepts every offer of the receiving `endpoint` at once and drops its
    /// data, for benchmarks.
    #[cfg(any(test, feature = "bench"))]
    #[cfg_attr(not(feature = "client"), allow(dead_code))]
    pub(crate) fn discard_offers(&self, endpoint: u64) {
        if let Some(ep) = self.0.state.lock().endpoints.get_mut(&endpoint) {
            ep.receivers = 1;
            ep.discard = true;
        }
    }

    /// Adds (`present`) or removes a receiver of the offers of `endpoint`.
    /// Removing the last one rejects an offer that is waiting.
    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    pub(crate) fn set_receiver(&self, endpoint: u64, present: bool) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            let Some(ep) = state.endpoints.get_mut(&endpoint) else {
                return;
            };
            if present {
                ep.receivers += 1;
            } else {
                ep.receivers = ep.receivers.saturating_sub(1);
            }
            let waiting = ep
                .current
                .as_ref()
                .filter(|t| t.phase == Phase::Offered && !t.taken)
                .map(|t| t.serial);
            if ep.receivers == 0
                && let Some(serial) = waiting
            {
                self.reject_locked(&mut state, endpoint, serial, "no receiver", None, &mut fx);
            }
        }
        self.apply(fx);
    }

    // Connection ---------------------------------------------------------

    /// Attaches a new connection; returns its generation for [`Self::pump`]
    /// and [`Self::detach`].
    pub(crate) fn attach(&self, lane: StateLane) -> u64 {
        let mut state = self.0.state.lock();
        state.generation += 1;
        let generation = state.generation;
        #[cfg(not(target_arch = "wasm32"))]
        let (jobs, jobs_rx) = mpsc::unbounded_channel();
        #[cfg(not(target_arch = "wasm32"))]
        {
            state.jobs = Some((generation, jobs_rx));
        }
        state.connection = Some(Connection {
            lane,
            generation,
            #[cfg(not(target_arch = "wasm32"))]
            jobs,
        });
        generation
    }

    /// Writes the chunks of sinks that block, for the connection of
    /// `generation`, until it is detached. Each batch runs on a blocking
    /// thread; credit goes back once it is written, so a slow disk slows the
    /// sender down.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn sink_worker(&self, generation: u64) {
        let jobs = {
            let mut state = self.0.state.lock();
            match state.jobs.take() {
                Some((jobs_generation, jobs)) if jobs_generation == generation => Some(jobs),
                other => {
                    state.jobs = other;
                    None
                }
            }
        };
        let Some(mut jobs) = jobs else {
            return;
        };
        let mut pending: Option<SinkJob> = None;
        loop {
            let first = match pending.take() {
                Some(job) => job,
                None => match jobs.recv().await {
                    Some(job) => job,
                    None => return,
                },
            };
            let (endpoint, serial) = (first.endpoint, first.serial);
            let mut len = first.data.len();
            let mut batch = vec![first.data];
            while len < SINK_BATCH {
                match jobs.try_recv() {
                    Ok(job) if job.endpoint == endpoint && job.serial == serial => {
                        len += job.data.len();
                        batch.push(job.data);
                    }
                    Ok(job) => {
                        pending = Some(job);
                        break;
                    }
                    Err(_) => break,
                }
            }

            let taken = {
                let mut state = self.0.state.lock();
                Self::current_mut(&mut state, endpoint, serial)
                    .filter(|t| t.phase == Phase::Receiving)
                    .and_then(|t| {
                        let finishing = t.consumed + len as u64 == t.total;
                        match t.sink.take() {
                            Some(Sink::Files { sink, single }) => Some((sink, single, finishing)),
                            other => {
                                t.sink = other;
                                None
                            }
                        }
                    })
            };
            let Some((mut sink, single, finishing)) = taken else {
                continue;
            };
            let written = tokio::task::spawn_blocking(move || {
                let mut result = Ok(());
                for data in &batch {
                    result = sink.write(data);
                    if result.is_err() {
                        break;
                    }
                }
                if result.is_ok() && finishing {
                    result = sink.finish();
                }
                (sink, result)
            })
            .await;

            let mut fx = Effects::default();
            {
                let mut state = self.0.state.lock();
                if state
                    .connection
                    .as_ref()
                    .is_none_or(|c| c.generation != generation)
                {
                    return;
                }
                match written {
                    Ok((sink, Ok(()))) => {
                        if let Some(t) = Self::current_mut(&mut state, endpoint, serial) {
                            t.sink = Some(Sink::Files { sink, single });
                            self.consumed(&mut state, endpoint, serial, len as u64, &mut fx);
                        }
                    }
                    Ok((sink, Err(error))) => {
                        // Removes the partial file.
                        drop(sink);
                        if Self::current_mut(&mut state, endpoint, serial).is_some() {
                            let error = TransferError::Io(error.to_string());
                            self.fail_locked(&mut state, endpoint, serial, error, &mut fx);
                        }
                    }
                    Err(error) => {
                        if Self::current_mut(&mut state, endpoint, serial).is_some() {
                            let error = TransferError::Io(format!("writing failed: {error}"));
                            self.fail_locked(&mut state, endpoint, serial, error, &mut fx);
                        }
                    }
                }
            }
            self.apply(fx);
        }
    }

    /// Detaches the connection of `generation`: every unfinished transfer
    /// fails as disconnected. Detaching an older generation does nothing.
    pub(crate) fn detach(&self, generation: u64) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            if state
                .connection
                .as_ref()
                .is_none_or(|c| c.generation != generation)
            {
                return;
            }
            state.connection = None;
            state.queue.clear();
            let ids: Vec<u64> = state.endpoints.keys().copied().collect();
            for id in ids {
                Self::finish_locked(
                    &mut state,
                    id,
                    TransferStatus::Failed(TransferError::Disconnected),
                    &mut fx,
                );
                // A send whose files are still being picked ends too; the files
                // must not go to whatever connection comes next.
                if let Some(reserved) = state
                    .endpoints
                    .get_mut(&id)
                    .and_then(|ep| ep.reserved.take())
                {
                    reserved.set_status(TransferStatus::Failed(TransferError::Disconnected));
                }
            }
            fx.wake_free = true;
            fx.wake_pump = true;
        }
        self.apply(fx);
    }

    /// Sends the data of accepted transfers into `data`, one chunk at a time
    /// and only within the receiver's credit, until the connection of
    /// `generation` is detached or `data` is closed.
    pub(crate) async fn pump(&self, generation: u64, data: mpsc::Sender<Vec<u8>>) {
        loop {
            let job = {
                let mut state = self.0.state.lock();
                if state
                    .connection
                    .as_ref()
                    .is_none_or(|c| c.generation != generation)
                {
                    return;
                }
                self.next_job(&mut state)
            };
            let Some((endpoint, serial, mut source, len)) = job else {
                self.0.pump.notified().await;
                continue;
            };

            let read = source.read(len).await;

            let mut fx = Effects::default();
            let frame = {
                let mut state = self.0.state.lock();
                if state
                    .connection
                    .as_ref()
                    .is_none_or(|c| c.generation != generation)
                {
                    return;
                }
                self.chunk_read(&mut state, endpoint, serial, source, len, read, &mut fx)
            };
            self.apply(fx);
            if let Some(frame) = frame
                && data.send(frame).await.is_err()
            {
                return;
            }
        }
    }

    /// Takes the source of the head transfer and the length of its next
    /// chunk, or `None` when nothing may be sent now.
    fn next_job(&self, state: &mut State) -> Option<(u64, u32, Source, usize)> {
        while let Some(&(endpoint, serial)) = state.queue.front() {
            let transfer = state
                .endpoints
                .get_mut(&endpoint)
                .and_then(|ep| ep.current.as_mut())
                .filter(|t| t.serial == serial);
            let Some(transfer) = transfer else {
                state.queue.pop_front();
                continue;
            };
            // The next transfer starts only once the receiver confirmed this
            // one: its last window may still be in flight, and starting the
            // next would put a second window on the connection.
            if transfer.phase != Phase::Sending {
                return None;
            }
            // The pump holds the source while it reads; it takes one chunk at a time.
            transfer.source.as_ref()?;
            let remaining = transfer.total - transfer.sent;
            let len = remaining
                .min(transfer.credit)
                .min(FILE_TRANSFER_CHUNK as u64);
            // An empty transfer still sends one empty last chunk.
            if len == 0 && remaining > 0 {
                return None;
            }
            transfer.credit -= len;
            transfer.shared.set_status(TransferStatus::Running);
            let source = transfer.source.take()?;
            return Some((endpoint, serial, source, len as usize));
        }
        None
    }

    /// Records a chunk the pump has read; returns the encoded chunk to send.
    #[allow(clippy::too_many_arguments)]
    fn chunk_read(
        &self,
        state: &mut State,
        endpoint: u64,
        serial: u32,
        source: Source,
        len: usize,
        read: Result<Vec<u8>, String>,
        fx: &mut Effects,
    ) -> Option<Vec<u8>> {
        // A transfer cancelled while its chunk was read is gone.
        Self::current_mut(state, endpoint, serial).filter(|t| t.phase == Phase::Sending)?;
        let chunk = match read {
            Ok(chunk) if chunk.len() == len => chunk,
            Ok(_) => {
                let error = TransferError::Io("the data changed size after the offer".to_string());
                self.fail_locked(state, endpoint, serial, error, fx);
                return None;
            }
            Err(error) => {
                self.fail_locked(state, endpoint, serial, TransferError::Io(error), fx);
                return None;
            }
        };
        let transfer = Self::current_mut(state, endpoint, serial).expect("checked above");
        transfer.source = Some(source);
        transfer.sent += len as u64;
        transfer.shared.set_done_bytes(transfer.sent);
        let last = transfer.sent == transfer.total;
        if last {
            // It leaves the queue once the receiver confirms it.
            transfer.phase = Phase::AwaitFinish;
            transfer.source = None;
        }
        fx.wake_pump = true;
        Some(self.encode(endpoint, &TransferFrame::Chunk { serial, last }, &chunk))
    }

    // Frames from the other peer ----------------------------------------------

    /// Handles a transfer frame from the connection's reader. It never waits:
    /// sinks that block are written by their own worker.
    pub(crate) fn handle_frame(&self, endpoint: u64, frame: TransferFrame, payload: Bytes) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            let Some(role) = state.endpoints.get(&endpoint).map(|ep| ep.role) else {
                if let TransferFrame::Offer { serial, .. } = frame {
                    self.emit(
                        &state,
                        endpoint,
                        &TransferFrame::Reject { serial },
                        b"unknown endpoint",
                    );
                }
                return;
            };
            match (role, frame) {
                (Role::Receive, TransferFrame::Offer { serial, meta_type }) => {
                    self.on_offer(&mut state, endpoint, serial, meta_type, &payload, &mut fx);
                }
                (Role::Receive, TransferFrame::Chunk { serial, last }) => {
                    self.on_chunk(&mut state, endpoint, serial, last, &payload, &mut fx);
                }
                (Role::Send, TransferFrame::Accept { serial, mode }) => {
                    self.on_accept(&mut state, endpoint, serial, mode, &mut fx);
                }
                (Role::Send, TransferFrame::Reject { serial }) => {
                    if self.current(&state, endpoint, serial, &[Phase::Offered]) {
                        let reason = String::from_utf8_lossy(&payload).into_owned();
                        Self::finish_locked(
                            &mut state,
                            endpoint,
                            TransferStatus::Failed(TransferError::Rejected(reason)),
                            &mut fx,
                        );
                    }
                }
                (Role::Send, TransferFrame::Credit { serial, bytes }) => {
                    if let Some(t) = Self::current_mut(&mut state, endpoint, serial)
                        && t.phase == Phase::Sending
                    {
                        t.credit += u64::from(bytes);
                        fx.wake_pump = true;
                    }
                }
                (Role::Send, TransferFrame::Finished { serial, error }) => {
                    if self.current(&state, endpoint, serial, &[]) {
                        let status = match error {
                            None if self.current(
                                &state,
                                endpoint,
                                serial,
                                &[Phase::AwaitFinish],
                            ) =>
                            {
                                TransferStatus::Done
                            }
                            None => TransferStatus::Failed(TransferError::Protocol(
                                "the receiver finished before all data was sent".to_string(),
                            )),
                            Some(code) => TransferStatus::Failed(TransferError::from_wire(
                                code,
                                String::from_utf8_lossy(&payload).into_owned(),
                            )),
                        };
                        Self::finish_locked(&mut state, endpoint, status, &mut fx);
                    }
                }
                (
                    Role::Receive,
                    TransferFrame::Finished {
                        serial,
                        error: None,
                    },
                ) if Self::current_mut(&mut state, endpoint, serial)
                    .is_some_and(|t| t.phase == Phase::Receiving && t.is_browser()) =>
                {
                    // The server sent every browser download.
                    Self::finish_locked(&mut state, endpoint, TransferStatus::Done, &mut fx);
                }
                (Role::Receive, TransferFrame::Finished { serial, error }) => {
                    if self.current(&state, endpoint, serial, &[]) {
                        let error = match error {
                            Some(code) => TransferError::from_wire(
                                code,
                                String::from_utf8_lossy(&payload).into_owned(),
                            ),
                            None => TransferError::Protocol(
                                "the sender confirmed its own transfer".to_string(),
                            ),
                        };
                        Self::finish_locked(
                            &mut state,
                            endpoint,
                            TransferStatus::Failed(error),
                            &mut fx,
                        );
                    }
                }
                (_, TransferFrame::Cancel { serial }) => {
                    if self.current(&state, endpoint, serial, &[]) {
                        let reason = (!payload.is_empty())
                            .then(|| String::from_utf8_lossy(&payload).into_owned());
                        Self::finish_locked(
                            &mut state,
                            endpoint,
                            TransferStatus::Failed(TransferError::Cancelled {
                                by_peer: true,
                                reason,
                            }),
                            &mut fx,
                        );
                    }
                }
                (
                    Role::Receive,
                    TransferFrame::Ticket {
                        serial,
                        file,
                        ticket,
                    },
                ) => {
                    if let Some(t) = Self::current_mut(&mut state, endpoint, serial)
                        .filter(|t| t.phase == Phase::Receiving && t.is_browser())
                    {
                        let name = match t.body.files.get(file as usize) {
                            Some(entry) => names::flatten(&entry.name),
                            None => zip_name(&t.body.name),
                        };
                        let ticket = ticket.iter().map(|b| format!("{b:02x}")).collect();
                        fx.downloads.push(BrowserDownload { ticket, name });
                    }
                }
                (
                    Role::Receive,
                    TransferFrame::HttpProgress {
                        serial,
                        done,
                        files_done: _,
                    },
                ) => {
                    if let Some(t) = Self::current_mut(&mut state, endpoint, serial)
                        .filter(|t| t.phase == Phase::Receiving && t.is_browser())
                    {
                        t.shared.set_done_bytes(done.min(t.total));
                        t.shared.set_status(TransferStatus::Running);
                    }
                }
                // A frame the endpoint's role does not use.
                (_, frame) => {
                    let serial = frame.serial();
                    if self.current(&state, endpoint, serial, &[]) {
                        self.fail_locked(
                            &mut state,
                            endpoint,
                            serial,
                            TransferError::Protocol(format!("unexpected frame {frame:?}")),
                            &mut fx,
                        );
                    }
                }
            }
        }
        self.apply(fx);
    }

    fn on_offer(
        &self,
        state: &mut State,
        endpoint: u64,
        serial: u32,
        meta_type: u32,
        payload: &[u8],
        fx: &mut Effects,
    ) {
        let reject = |state: &State, reason: &str| {
            self.emit(
                state,
                endpoint,
                &TransferFrame::Reject { serial },
                reason.as_bytes(),
            );
        };
        let (body, meta) = match decode_offer(payload) {
            Ok(decoded) => decoded,
            Err(error) => return reject(state, &error),
        };
        // The same limits a local send checks, so a peer cannot make this
        // endpoint hold more.
        if meta.len() > VALUE_MAX_SIZE || payload.len() - meta.len() > VALUE_MAX_SIZE {
            return reject(state, "the offer is too large");
        }
        if let Err(error) = names::validate_offer(&body) {
            return reject(state, &format!("invalid offer: {error}"));
        }
        let key = state.next_key;
        let ep = state.endpoints.get_mut(&endpoint).expect("endpoint exists");
        if ep.meta_type != meta_type {
            return reject(state, "the metadata type does not match");
        }
        if ep.current.is_some() {
            return reject(state, "the endpoint already has an unfinished transfer");
        }
        if ep.receivers == 0 {
            return reject(state, "no receiver");
        }
        let shared =
            TransferShared::new(self.0.this.clone(), endpoint, key, self.0.repaint.clone());
        let meta = Bytes::copy_from_slice(meta);
        shared.bind(
            &body.name,
            body.files.iter().map(|f| f.size).collect(),
            meta.clone(),
        );
        #[cfg(any(test, feature = "bench"))]
        if ep.discard {
            let mut transfer = Transfer::new(serial, shared, body, meta);
            transfer.phase = Phase::Receiving;
            transfer.sink = Some(Sink::Discard);
            transfer.granted = transfer.total.min(FILE_TRANSFER_WINDOW);
            ep.current = Some(transfer);
            state.next_key += 1;
            let mode = AcceptMode::Lane;
            self.emit(
                state,
                endpoint,
                &TransferFrame::Accept { serial, mode },
                &[],
            );
            return;
        }
        ep.current = Some(Transfer::new(serial, shared, body, meta));
        state.next_key += 1;
        fx.events.push(EngineEvent::Offer { endpoint });
        if let Some(repaint) = &self.0.repaint {
            repaint(None);
        }
    }

    fn on_chunk(
        &self,
        state: &mut State,
        endpoint: u64,
        serial: u32,
        last: bool,
        payload: &Bytes,
        fx: &mut Effects,
    ) {
        let Some(t) = Self::current_mut(state, endpoint, serial) else {
            // A late chunk of a cancelled transfer.
            return;
        };
        if t.phase != Phase::Receiving {
            let error = TransferError::Protocol("data arrived before the accept".to_string());
            self.fail_locked(state, endpoint, serial, error, fx);
            return;
        }
        let len = payload.len() as u64;
        let error = if t.received + len > t.total {
            Some("more data than announced")
        } else if t.received + len > t.granted {
            Some("data beyond the granted credit")
        } else if last != (t.received + len == t.total) {
            Some("the last chunk does not end the data")
        } else {
            None
        };
        if let Some(error) = error {
            let error = TransferError::Protocol(error.to_string());
            self.fail_locked(state, endpoint, serial, error, fx);
            return;
        }
        t.received += len;
        t.shared.set_status(TransferStatus::Running);
        let Some(sink) = t.sink.as_mut() else {
            // The sink worker holds the sink; it takes the chunk in order.
            #[cfg(not(target_arch = "wasm32"))]
            self.queue_job(state, endpoint, serial, payload);
            return;
        };
        if sink.is_inline() {
            if let Err(error) = sink.write_inline(payload) {
                self.fail_locked(state, endpoint, serial, error, fx);
                return;
            }
            self.consumed(state, endpoint, serial, len, fx);
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            self.queue_job(state, endpoint, serial, payload);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn queue_job(&self, state: &State, endpoint: u64, serial: u32, payload: &Bytes) {
        if let Some(connection) = &state.connection {
            let _ = connection.jobs.send(SinkJob {
                endpoint,
                serial,
                data: payload.clone(),
            });
        }
    }

    /// Records that the sink took `len` more bytes: returns credit and, once
    /// everything is in, completes the transfer.
    fn consumed(&self, state: &mut State, endpoint: u64, serial: u32, len: u64, fx: &mut Effects) {
        let Some(t) = Self::current_mut(state, endpoint, serial) else {
            return;
        };
        t.consumed += len;
        t.shared.set_done_bytes(t.consumed);
        if t.consumed == t.total {
            if let Some(data) = t.sink.take().and_then(Sink::into_data) {
                t.shared.set_data(data);
            }
            self.emit(
                state,
                endpoint,
                &TransferFrame::Finished {
                    serial,
                    error: None,
                },
                &[],
            );
            Self::finish_locked(state, endpoint, TransferStatus::Done, fx);
            return;
        }
        if let Some(bytes) = grant(t) {
            self.emit(
                state,
                endpoint,
                &TransferFrame::Credit { serial, bytes },
                &[],
            );
        }
    }

    fn on_accept(
        &self,
        state: &mut State,
        endpoint: u64,
        serial: u32,
        mode: AcceptMode,
        fx: &mut Effects,
    ) {
        if !self.current(state, endpoint, serial, &[Phase::Offered]) {
            return;
        }
        if mode != AcceptMode::Lane {
            #[cfg(any(feature = "server", feature = "python"))]
            self.start_http(state, endpoint, serial, mode == AcceptMode::HttpBundled, fx);
            #[cfg(not(any(feature = "server", feature = "python")))]
            {
                let error = TransferError::Unsupported("browser downloads need a server".into());
                self.fail_locked(state, endpoint, serial, error, fx);
            }
            return;
        }
        let t = Self::current_mut(state, endpoint, serial).expect("checked above");
        t.phase = Phase::Sending;
        t.credit = t.total.min(FILE_TRANSFER_WINDOW);
        state.queue.push_back((endpoint, serial));
        fx.wake_pump = true;
    }

    // Sending ------------------------------------------------------------------

    /// Offers `body` on a sending endpoint; the data comes from `source`.
    ///
    /// Fails without effect when no connection is attached, when the endpoint
    /// is busy, or when the offer is invalid.
    pub(crate) fn send(
        &self,
        endpoint: u64,
        body: OfferBody,
        meta: Bytes,
        source: Source,
    ) -> Result<Arc<TransferShared>, TransferError> {
        let payload = Self::offer_payload(&body, &meta)?;
        let mut state = self.0.state.lock();
        let ep = Self::sending_endpoint(&mut state, endpoint)?;
        if ep.is_busy() || !ep.waiters.is_empty() {
            return Err(TransferError::Busy);
        }
        Ok(self.offer_locked(&mut state, endpoint, None, body, meta, source, &payload))
    }

    /// Like [`Self::send`], but waits for the endpoint to become free, up to
    /// `timeout`. Threads that wait are served in the order they called.
    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn send_wait(
        &self,
        endpoint: u64,
        body: OfferBody,
        meta: Bytes,
        source: Source,
        timeout: Option<Duration>,
    ) -> Result<Arc<TransferShared>, TransferError> {
        let payload = Self::offer_payload(&body, &meta)?;
        let deadline = timeout.map(|timeout| std::time::Instant::now() + timeout);
        let mut state = self.0.state.lock();
        Self::sending_endpoint(&mut state, endpoint)?;
        let ticket = state.next_waiter;
        state.next_waiter += 1;
        let leave = |state: &mut State| {
            if let Some(ep) = state.endpoints.get_mut(&endpoint) {
                ep.waiters.retain(|waiter| *waiter != ticket);
            }
        };
        state
            .endpoints
            .get_mut(&endpoint)
            .expect("checked above")
            .waiters
            .push_back(ticket);
        loop {
            if state.connection.is_none() {
                leave(&mut state);
                drop(state);
                self.0.free.notify_all();
                return Err(TransferError::NotConnected);
            }
            let ep = state.endpoints.get(&endpoint).expect("checked above");
            if !ep.is_busy() && ep.waiters.front() == Some(&ticket) {
                break;
            }
            let timed_out = match deadline {
                Some(deadline) => self.0.free.wait_until(&mut state, deadline).timed_out(),
                None => {
                    self.0.free.wait(&mut state);
                    false
                }
            };
            if timed_out {
                let ep = state.endpoints.get(&endpoint).expect("checked above");
                if !(state.connection.is_some()
                    && !ep.is_busy()
                    && ep.waiters.front() == Some(&ticket))
                {
                    leave(&mut state);
                    drop(state);
                    self.0.free.notify_all();
                    return Err(TransferError::Timeout);
                }
            }
        }
        leave(&mut state);
        let shared = self.offer_locked(&mut state, endpoint, None, body, meta, source, &payload);
        drop(state);
        self.0.free.notify_all();
        Ok(shared)
    }

    /// Makes the endpoint busy while the files of a send are picked; the
    /// handle stays pending until [`Self::send_reserved`] or
    /// [`Self::release`].
    #[cfg_attr(not(feature = "client"), allow(dead_code))]
    pub(crate) fn reserve(&self, endpoint: u64) -> Result<Arc<TransferShared>, TransferError> {
        let mut state = self.0.state.lock();
        let key = state.next_key;
        let ep = Self::sending_endpoint(&mut state, endpoint)?;
        if ep.is_busy() || !ep.waiters.is_empty() {
            return Err(TransferError::Busy);
        }
        let shared =
            TransferShared::new(self.0.this.clone(), endpoint, key, self.0.repaint.clone());
        ep.reserved = Some(shared.clone());
        ep.last = Some(shared.clone());
        state.next_key += 1;
        Ok(shared)
    }

    /// Offers the files picked for a reserved send. A failure ends the
    /// handle.
    #[cfg_attr(not(feature = "client"), allow(dead_code))]
    pub(crate) fn send_reserved(
        &self,
        shared: &Arc<TransferShared>,
        body: OfferBody,
        meta: Bytes,
        source: Source,
    ) {
        let endpoint = shared.endpoint;
        let payload = match Self::offer_payload(&body, &meta) {
            Ok(payload) => payload,
            Err(error) => return self.release(shared, error),
        };
        let mut state = self.0.state.lock();
        let reserved = state
            .endpoints
            .get(&endpoint)
            .and_then(|ep| ep.reserved.as_ref())
            .is_some_and(|reserved| Arc::ptr_eq(reserved, shared));
        if !reserved {
            // Cancelled while the files were picked.
            return;
        }
        if state.connection.is_none() {
            drop(state);
            return self.release(shared, TransferError::NotConnected);
        }
        state
            .endpoints
            .get_mut(&endpoint)
            .expect("reserved")
            .reserved = None;
        self.offer_locked(
            &mut state,
            endpoint,
            Some(shared.clone()),
            body,
            meta,
            source,
            &payload,
        );
    }

    /// Ends a reserved send without an offer.
    #[cfg_attr(not(feature = "client"), allow(dead_code))]
    pub(crate) fn release(&self, shared: &Arc<TransferShared>, error: TransferError) {
        {
            let mut state = self.0.state.lock();
            if let Some(ep) = state.endpoints.get_mut(&shared.endpoint)
                && ep
                    .reserved
                    .as_ref()
                    .is_some_and(|reserved| Arc::ptr_eq(reserved, shared))
            {
                ep.reserved = None;
            }
        }
        shared.set_status(TransferStatus::Failed(error));
        self.0.free.notify_all();
    }

    fn offer_payload(body: &OfferBody, meta: &[u8]) -> Result<Vec<u8>, TransferError> {
        names::validate_offer(body).map_err(TransferError::InvalidInput)?;
        if meta.len() > VALUE_MAX_SIZE {
            return Err(TransferError::Meta(format!(
                "{} bytes, at most {VALUE_MAX_SIZE}",
                meta.len()
            )));
        }
        let body_len = postcard::experimental::serialized_size(body)
            .map_err(|e| TransferError::InvalidInput(e.to_string()))?;
        if body_len > VALUE_MAX_SIZE {
            return Err(TransferError::InvalidInput(
                "too many files for one transfer".to_string(),
            ));
        }
        encode_offer(body, meta).map_err(TransferError::InvalidInput)
    }

    fn sending_endpoint(state: &mut State, endpoint: u64) -> Result<&mut Endpoint, TransferError> {
        if state.connection.is_none() {
            return Err(TransferError::NotConnected);
        }
        match state.endpoints.get_mut(&endpoint) {
            Some(ep) if ep.role == Role::Send => Ok(ep),
            _ => Err(TransferError::InvalidInput(
                "the endpoint does not send".to_string(),
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn offer_locked(
        &self,
        state: &mut State,
        endpoint: u64,
        shared: Option<Arc<TransferShared>>,
        body: OfferBody,
        meta: Bytes,
        source: Source,
        payload: &[u8],
    ) -> Arc<TransferShared> {
        let shared = shared.unwrap_or_else(|| {
            let key = state.next_key;
            state.next_key += 1;
            TransferShared::new(self.0.this.clone(), endpoint, key, self.0.repaint.clone())
        });
        shared.bind(
            &body.name,
            body.files.iter().map(|f| f.size).collect(),
            meta.clone(),
        );
        let ep = state
            .endpoints
            .get_mut(&endpoint)
            .expect("checked by caller");
        let serial = ep.next_serial;
        ep.next_serial = ep.next_serial.wrapping_add(1);
        let meta_type = ep.meta_type;
        let mut transfer = Transfer::new(serial, shared.clone(), body, meta);
        transfer.source = Some(source);
        transfer.exposed = true;
        ep.current = Some(transfer);
        ep.last = Some(shared.clone());
        self.emit(
            state,
            endpoint,
            &TransferFrame::Offer { serial, meta_type },
            payload,
        );
        shared
    }

    // Receiving ----------------------------------------------------------------

    /// Hands the waiting offer of `endpoint` to the application, once.
    pub(crate) fn take_offer(&self, endpoint: u64) -> Option<RawOffer> {
        let mut state = self.0.state.lock();
        let t = state
            .endpoints
            .get_mut(&endpoint)?
            .current
            .as_mut()
            .filter(|t| t.phase == Phase::Offered && !t.taken)?;
        t.taken = true;
        Some(RawOffer {
            token: OfferToken {
                engine: self.clone(),
                endpoint,
                serial: t.serial,
                shared: t.shared.clone(),
                answered: false,
            },
            body: t.body.clone(),
            meta: t.meta.clone(),
        })
    }

    /// The most recent transfer of `endpoint` the application has a handle
    /// for.
    pub(crate) fn last(&self, endpoint: u64) -> Option<Arc<TransferShared>> {
        self.0.state.lock().endpoints.get(&endpoint)?.last.clone()
    }

    /// Whether `endpoint` has an unfinished transfer or a send being picked.
    #[cfg_attr(not(feature = "client"), allow(dead_code))]
    pub(crate) fn is_busy(&self, endpoint: u64) -> bool {
        self.0
            .state
            .lock()
            .endpoints
            .get(&endpoint)
            .is_some_and(Endpoint::is_busy)
    }

    /// Cancels the transfer of `shared`.
    pub(crate) fn cancel(&self, shared: &Arc<TransferShared>, reason: Option<String>) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            let endpoint = shared.endpoint;
            let Some(ep) = state.endpoints.get_mut(&endpoint) else {
                drop(state);
                shared.set_status(cancelled(reason));
                return;
            };
            if ep.reserved.as_ref().is_some_and(|r| Arc::ptr_eq(r, shared)) {
                ep.reserved = None;
                shared.set_status(cancelled(reason));
                fx.wake_free = true;
            } else if let Some(serial) = ep
                .current
                .as_ref()
                .filter(|t| Arc::ptr_eq(&t.shared, shared))
                .map(|t| t.serial)
            {
                let payload = reason.clone().unwrap_or_default();
                self.emit(
                    &state,
                    endpoint,
                    &TransferFrame::Cancel { serial },
                    payload.as_bytes(),
                );
                Self::finish_locked(&mut state, endpoint, cancelled(reason), &mut fx);
            }
        }
        self.apply(fx);
    }

    // Helpers -------------------------------------------------------------------

    fn current(&self, state: &State, endpoint: u64, serial: u32, phases: &[Phase]) -> bool {
        state
            .endpoints
            .get(&endpoint)
            .and_then(|ep| ep.current.as_ref())
            .is_some_and(|t| t.serial == serial && (phases.is_empty() || phases.contains(&t.phase)))
    }

    fn current_mut(state: &mut State, endpoint: u64, serial: u32) -> Option<&mut Transfer> {
        state
            .endpoints
            .get_mut(&endpoint)?
            .current
            .as_mut()
            .filter(|t| t.serial == serial)
    }

    /// Ends the current transfer of `endpoint` with `status`.
    fn finish_locked(
        state: &mut State,
        endpoint: u64,
        status: TransferStatus,
        fx: &mut Effects,
    ) -> Option<Transfer> {
        let ep = state.endpoints.get_mut(&endpoint)?;
        let transfer = ep.current.take()?;
        transfer.shared.set_status(status);
        if transfer.exposed {
            fx.events.push(EngineEvent::Finished {
                endpoint,
                handle: transfer.shared.clone(),
            });
        }
        let serial = transfer.serial;
        state.queue.retain(|entry| *entry != (endpoint, serial));
        fx.wake_free = true;
        fx.wake_pump = true;
        Some(transfer)
    }

    /// Fails the current transfer and tells the other side why.
    fn fail_locked(
        &self,
        state: &mut State,
        endpoint: u64,
        serial: u32,
        error: TransferError,
        fx: &mut Effects,
    ) {
        let (code, message) = error.wire();
        self.emit(
            state,
            endpoint,
            &TransferFrame::Finished {
                serial,
                error: Some(code as u8),
            },
            message.as_bytes(),
        );
        Self::finish_locked(state, endpoint, TransferStatus::Failed(error), fx);
    }

    /// Rejects an offer; the local handle, if any, ends with `local`, or as
    /// rejected.
    fn reject_locked(
        &self,
        state: &mut State,
        endpoint: u64,
        serial: u32,
        reason: &str,
        local: Option<TransferError>,
        fx: &mut Effects,
    ) {
        self.emit(
            state,
            endpoint,
            &TransferFrame::Reject { serial },
            reason.as_bytes(),
        );
        let error = local.unwrap_or_else(|| TransferError::Rejected(reason.to_string()));
        Self::finish_locked(state, endpoint, TransferStatus::Failed(error), fx);
    }

    fn encode(&self, endpoint: u64, frame: &TransferFrame, payload: &[u8]) -> Vec<u8> {
        match self.0.side {
            Side::Server => encode_server(endpoint, frame, payload),
            Side::Client => encode_client(endpoint, frame, payload),
        }
    }

    /// Sends a control frame into the state lane, if a connection is attached.
    fn emit(&self, state: &State, endpoint: u64, frame: &TransferFrame, payload: &[u8]) {
        if let Some(connection) = &state.connection {
            (connection.lane)(self.encode(endpoint, frame, payload));
        }
    }

    fn apply(&self, fx: Effects) {
        if let Some(hook) = self.0.tickets.get() {
            for download in fx.downloads {
                hook(download);
            }
        }
        if fx.wake_pump {
            self.0.pump.notify_one();
        }
        if fx.wake_free {
            self.0.free.notify_all();
        }
        if !fx.events.is_empty()
            && let Some(hook) = self.0.events.get()
        {
            for event in fx.events {
                hook(event);
            }
        }
    }
}

/// The name of the zip a set of files is bundled into.
fn zip_name(set: &str) -> String {
    let flat = names::flatten(set);
    if flat.to_ascii_lowercase().ends_with(".zip") {
        flat
    } else {
        format!("{flat}.zip")
    }
}

fn cancelled(reason: Option<String>) -> TransferStatus {
    TransferStatus::Failed(TransferError::Cancelled {
        by_peer: false,
        reason,
    })
}

/// The credit to return once the receiver's outstanding grant has fallen to
/// half a window, if any.
fn grant(t: &mut Transfer) -> Option<u32> {
    if t.granted >= t.total || t.granted - t.consumed > FILE_TRANSFER_WINDOW / 2 {
        return None;
    }
    let granted = (t.consumed + FILE_TRANSFER_WINDOW).min(t.total);
    let bytes = granted - t.granted;
    t.granted = granted;
    Some(bytes as u32)
}

impl OfferToken {
    /// The handle the transfer will have.
    #[cfg_attr(any(not(feature = "client"), target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn shared(&self) -> &Arc<TransferShared> {
        &self.shared
    }

    /// Whether the endpoint still holds this offer, unanswered. A serial is
    /// reused by the next connection, so the transfer itself is compared.
    fn is_open(&self, state: &State) -> bool {
        state.connection.is_some()
            && state
                .endpoints
                .get(&self.endpoint)
                .and_then(|ep| ep.current.as_ref())
                .is_some_and(|t| {
                    t.serial == self.serial
                        && t.phase == Phase::Offered
                        && Arc::ptr_eq(&t.shared, &self.shared)
                })
    }

    /// Whether the offer was withdrawn by the sender or lost to a disconnect.
    #[cfg_attr(not(feature = "client"), allow(dead_code))]
    pub(crate) fn is_cancelled(&self) -> bool {
        let state = self.engine.0.state.lock();
        !self.is_open(&state)
    }

    /// Makes the handle the endpoint's last one while the destination is
    /// still being picked.
    #[cfg_attr(any(not(feature = "client"), target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn expose(&self) {
        let mut state = self.engine.0.state.lock();
        let Some(ep) = state.endpoints.get_mut(&self.endpoint) else {
            return;
        };
        if let Some(t) = ep
            .current
            .as_mut()
            .filter(|t| Arc::ptr_eq(&t.shared, &self.shared))
        {
            t.exposed = true;
            ep.last = Some(self.shared.clone());
        }
    }

    /// Accepts the offer into `sink`. An offer the sink cannot take is
    /// rejected and the handle fails; an offer that was cancelled meanwhile
    /// returns its failed handle.
    pub(crate) fn accept(
        mut self,
        sink: Sink,
        mode: AcceptMode,
        path: Option<PathBuf>,
    ) -> Arc<TransferShared> {
        self.answered = true;
        let engine = self.engine.clone();
        let mut fx = Effects::default();
        {
            let mut state = engine.0.state.lock();
            if !self.is_open(&state) {
                drop(state);
                if !self.shared.is_finished() {
                    self.shared
                        .set_status(TransferStatus::Failed(TransferError::Disconnected));
                }
                return self.shared.clone();
            }
            if let Some(ep) = state.endpoints.get_mut(&self.endpoint) {
                ep.last = Some(self.shared.clone());
            }
            let t = Self::transfer(&mut state, self.endpoint, self.serial);
            t.exposed = true;
            let check = sink
                .check(t.body.files.len(), t.total)
                .and_then(|()| names::check_platform(&t.body).map_err(TransferError::Unsupported));
            if let Err(error) = check {
                let reason = error.to_string();
                engine.reject_locked(
                    &mut state,
                    self.endpoint,
                    self.serial,
                    &reason,
                    Some(error),
                    &mut fx,
                );
            } else {
                t.phase = Phase::Receiving;
                t.sink = Some(sink);
                t.granted = t.total.min(FILE_TRANSFER_WINDOW);
                if let Some(path) = path {
                    t.shared.set_path(path);
                }
                engine.emit(
                    &state,
                    self.endpoint,
                    &TransferFrame::Accept {
                        serial: self.serial,
                        mode,
                    },
                    &[],
                );
            }
        }
        engine.apply(fx);
        self.shared.clone()
    }

    /// Rejects the offer; the handle, if the application holds one, ends with
    /// `local`.
    pub(crate) fn reject(mut self, reason: &str, local: Option<TransferError>) {
        self.answered = true;
        self.reject_inner(reason, local);
    }

    /// Fails the offer with `error` and returns the failed handle.
    #[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
    pub(crate) fn fail(mut self, error: TransferError) -> Arc<TransferShared> {
        self.answered = true;
        self.expose();
        let reason = error.to_string();
        self.reject_inner(&reason, Some(error));
        self.shared.clone()
    }

    fn reject_inner(&self, reason: &str, local: Option<TransferError>) {
        let mut fx = Effects::default();
        {
            let mut state = self.engine.0.state.lock();
            if self.is_open(&state) {
                self.engine.reject_locked(
                    &mut state,
                    self.endpoint,
                    self.serial,
                    reason,
                    local,
                    &mut fx,
                );
            }
        }
        self.engine.apply(fx);
    }

    fn transfer(state: &mut State, endpoint: u64, serial: u32) -> &mut Transfer {
        Engine::current_mut(state, endpoint, serial).expect("checked by caller")
    }
}

impl Drop for OfferToken {
    fn drop(&mut self) {
        if !self.answered {
            self.reject_inner(NOT_ACCEPTED, None);
        }
    }
}

#[cfg(any(feature = "server", feature = "python"))]
mod http;
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) use http::HttpJob;
#[cfg(test)]
mod tests;
