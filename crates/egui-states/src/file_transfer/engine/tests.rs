//! Two engines joined by in-memory lanes, as a server and a client would be.

use std::time::{Duration, Instant};

use tokio::runtime::Runtime;
use tokio::sync::mpsc::unbounded_channel;

use super::*;
use crate::protocol::file_transfer::FileEntry;
use crate::protocol::{ClientHeader, ServerHeader};

/// Server sends on it, the client receives.
const DOWN: u64 = 100;
/// A second download endpoint.
const DOWN2: u64 = 101;
/// The client sends on it, the server receives.
const UP: u64 = 200;
const DEADLINE: Duration = Duration::from_secs(10);

fn decode(from: Side, data: &[u8]) -> (u64, TransferFrame, Bytes) {
    let (endpoint, frame, len, rest) = match from {
        Side::Server => match postcard::take_from_bytes::<ServerHeader>(data).unwrap() {
            (ServerHeader::Transfer(endpoint, frame, len), rest) => (endpoint, frame, len, rest),
            _ => panic!("not a transfer message"),
        },
        Side::Client => match postcard::take_from_bytes::<ClientHeader>(data).unwrap() {
            (ClientHeader::Transfer(endpoint, frame, len), rest) => (endpoint, frame, len, rest),
            _ => panic!("not a transfer message"),
        },
    };
    assert_eq!(rest.len(), len as usize, "payload length");
    (endpoint, frame, Bytes::copy_from_slice(rest))
}

struct Link {
    runtime: Runtime,
    server: Engine,
    client: Engine,
    generations: Option<(u64, u64)>,
}

impl Link {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .unwrap();
        let server = Engine::new(Side::Server, None);
        let client = Engine::new(Side::Client, None);
        for (endpoint, server_role) in
            [(DOWN, Role::Send), (DOWN2, Role::Send), (UP, Role::Receive)]
        {
            let client_role = match server_role {
                Role::Send => Role::Receive,
                Role::Receive => Role::Send,
            };
            server.register(endpoint, server_role, 0);
            client.register(endpoint, client_role, 0);
        }
        server.set_receiver(UP, true);
        Self {
            runtime,
            server,
            client,
            generations: None,
        }
    }

    /// Joins both engines: a state lane and a data lane in each direction.
    fn connect(&mut self) {
        let server_gen = self.join(Side::Server);
        let client_gen = self.join(Side::Client);
        self.generations = Some((server_gen, client_gen));
    }

    fn join(&self, from: Side) -> u64 {
        let (sender, receiver) = match from {
            Side::Server => (self.server.clone(), self.client.clone()),
            Side::Client => (self.client.clone(), self.server.clone()),
        };
        let (state_tx, mut state_rx) = unbounded_channel::<Vec<u8>>();
        let generation = sender.attach(Arc::new(move |message| {
            let _ = state_tx.send(message);
        }));
        let (data_tx, mut data_rx) = mpsc::channel::<Vec<u8>>(1);
        let pump = sender.clone();
        self.runtime
            .spawn(async move { pump.pump(generation, data_tx).await });
        let peer = receiver.clone();
        self.runtime.spawn(async move {
            while let Some(message) = state_rx.recv().await {
                let (endpoint, frame, payload) = decode(from, &message);
                peer.handle_frame(endpoint, frame, payload);
            }
        });
        self.runtime.spawn(async move {
            while let Some(message) = data_rx.recv().await {
                let (endpoint, frame, payload) = decode(from, &message);
                receiver.handle_frame(endpoint, frame, payload);
            }
        });
        generation
    }

    fn disconnect(&mut self) {
        let (server_gen, client_gen) = self.generations.take().unwrap();
        self.server.detach(server_gen);
        self.client.detach(client_gen);
    }

    /// Takes the next offer on the client's `endpoint`.
    fn client_offer(&self, endpoint: u64) -> RawOffer {
        until(|| self.client.take_offer(endpoint)).expect("an offer arrives")
    }
}

fn until<T>(mut check: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if let Some(value) = check() {
            return Some(value);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

fn finished(shared: &Arc<TransferShared>) -> TransferStatus {
    until(|| shared.is_finished().then(|| shared.status())).expect("the transfer finishes")
}

fn single(name: &str, size: u64) -> OfferBody {
    OfferBody {
        name: name.to_string(),
        files: vec![FileEntry {
            name: name.to_string(),
            size,
            modified: None,
        }],
    }
}

fn pattern(size: u64) -> Source {
    Source::Pattern { remaining: size }
}

fn send_bytes(engine: &Engine, endpoint: u64, data: Vec<u8>) -> Arc<TransferShared> {
    let body = single("data.bin", data.len() as u64);
    engine
        .send(endpoint, body, Bytes::new(), Source::bytes(data))
        .unwrap()
}

#[test]
fn download_into_memory_spans_several_credit_windows() {
    let mut link = Link::new();
    link.connect();
    let data: Vec<u8> = (0..(3 * FILE_TRANSFER_WINDOW + 123) as usize)
        .map(|i| (i % 251) as u8)
        .collect();
    let sent = send_bytes(&link.server, DOWN, data.clone());
    assert_eq!(sent.status(), TransferStatus::Pending);
    assert!(link.server.is_busy(DOWN));

    let offer = link.client_offer(DOWN);
    assert_eq!(offer.body.name, "data.bin");
    let received = offer
        .token
        .accept(Sink::memory(u64::MAX), AcceptMode::Lane, None);

    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(finished(&sent), TransferStatus::Done);
    assert_eq!(received.take_data().unwrap(), data);
    assert!(received.take_data().is_none(), "taken once");
    assert_eq!(sent.progress().done, data.len() as u64);
    assert!(!link.server.is_busy(DOWN));
    assert!(Arc::ptr_eq(&link.client.last(DOWN).unwrap(), &received));
}

#[test]
fn upload_into_memory() {
    let mut link = Link::new();
    link.connect();
    let sent = send_bytes(&link.client, UP, b"hello".to_vec());
    let offer = until(|| link.server.take_offer(UP)).unwrap();
    let received = offer.token.accept(Sink::memory(5), AcceptMode::Lane, None);
    assert_eq!(finished(&sent), TransferStatus::Done);
    assert_eq!(received.take_data().unwrap(), b"hello");
}

#[test]
fn several_files_and_empty_files_are_split_correctly() {
    let mut link = Link::new();
    link.connect();
    let sizes = [0, 70_000, 0, 1, 300_000, 0];
    let body = OfferBody {
        name: "set".to_string(),
        files: sizes
            .iter()
            .enumerate()
            .map(|(i, &size)| FileEntry {
                name: format!("dir/{i}.bin"),
                size,
                modified: Some(i as i64),
            })
            .collect(),
    };
    let total: u64 = sizes.iter().sum();
    let sent = link
        .server
        .send(DOWN, body, Bytes::new(), pattern(total))
        .unwrap();
    let offer = link.client_offer(DOWN);
    assert_eq!(offer.body.files.len(), sizes.len());
    let received = offer.token.accept(Sink::Discard, AcceptMode::Lane, None);
    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(finished(&sent), TransferStatus::Done);
    let progress = received.progress();
    assert_eq!(progress.done, total);
    assert_eq!(progress.files_done, sizes.len() as u32);
    assert_eq!(progress.files_total, sizes.len() as u32);
}

#[test]
fn an_empty_transfer_completes() {
    let mut link = Link::new();
    link.connect();
    let sent = send_bytes(&link.server, DOWN, Vec::new());
    let offer = link.client_offer(DOWN);
    let received = offer.token.accept(Sink::memory(0), AcceptMode::Lane, None);
    assert_eq!(finished(&sent), TransferStatus::Done);
    assert_eq!(received.take_data().unwrap(), Vec::<u8>::new());
}

#[test]
fn reject_and_dropped_offers_reach_the_sender() {
    let mut link = Link::new();
    link.connect();
    let sent = send_bytes(&link.server, DOWN, vec![1; 10]);
    link.client_offer(DOWN).token.reject("no thanks", None);
    assert_eq!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Rejected("no thanks".to_string()))
    );

    let sent = send_bytes(&link.server, DOWN, vec![1; 10]);
    drop(link.client_offer(DOWN));
    assert_eq!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Rejected(NOT_ACCEPTED.to_string()))
    );
    assert!(!link.server.is_busy(DOWN));
}

#[test]
fn an_accept_the_sink_cannot_take_rejects_with_the_reason() {
    let mut link = Link::new();
    link.connect();
    let sent = send_bytes(&link.server, DOWN, vec![1; 10]);
    let received = link
        .client_offer(DOWN)
        .token
        .accept(Sink::memory(9), AcceptMode::Lane, None);
    assert_eq!(
        received.status(),
        TransferStatus::Failed(TransferError::TooLarge)
    );
    assert_eq!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Rejected(TransferError::TooLarge.to_string()))
    );
}

#[test]
fn either_side_can_cancel_a_running_transfer() {
    for receiver_cancels in [false, true] {
        let mut link = Link::new();
        link.connect();
        let body = single("big.bin", 1 << 30);
        let sent = link
            .server
            .send(DOWN, body, Bytes::new(), pattern(1 << 30))
            .unwrap();
        let received = link
            .client_offer(DOWN)
            .token
            .accept(Sink::Discard, AcceptMode::Lane, None);
        until(|| (received.progress().done > 0).then_some(())).unwrap();
        let (canceller, other) = if receiver_cancels {
            (&received, &sent)
        } else {
            (&sent, &received)
        };
        canceller.cancel(Some("stop".to_string()));
        assert_eq!(
            finished(canceller),
            TransferStatus::Failed(TransferError::Cancelled {
                by_peer: false,
                reason: Some("stop".to_string())
            })
        );
        assert_eq!(
            finished(other),
            TransferStatus::Failed(TransferError::Cancelled {
                by_peer: true,
                reason: Some("stop".to_string())
            })
        );
        until(|| (!link.server.is_busy(DOWN) && !link.client.is_busy(DOWN)).then_some(())).unwrap();
    }
}

#[test]
fn an_endpoint_carries_one_transfer_at_a_time() {
    let mut link = Link::new();
    link.connect();
    let first = send_bytes(&link.server, DOWN, vec![1; 10]);
    assert_eq!(
        link.server
            .send(DOWN, single("b", 1), Bytes::new(), Source::bytes(vec![1]))
            .unwrap_err(),
        TransferError::Busy
    );
    link.client_offer(DOWN)
        .token
        .accept(Sink::memory(10), AcceptMode::Lane, None);
    assert_eq!(finished(&first), TransferStatus::Done);
    let second = until(|| {
        link.server
            .send(DOWN, single("b", 1), Bytes::new(), Source::bytes(vec![1]))
            .ok()
    })
    .unwrap();
    assert!(!second.is_finished());
}

#[test]
fn a_second_endpoint_waits_behind_the_running_transfer() {
    let mut link = Link::new();
    link.connect();
    let big = 64 * FILE_TRANSFER_WINDOW;
    let first = link
        .server
        .send(DOWN, single("big", big), Bytes::new(), pattern(big))
        .unwrap();
    let second = send_bytes(&link.server, DOWN2, vec![7; 10]);
    let first_rx = link
        .client_offer(DOWN)
        .token
        .accept(Sink::Discard, AcceptMode::Lane, None);
    let second_rx = link
        .client_offer(DOWN2)
        .token
        .accept(Sink::memory(10), AcceptMode::Lane, None);

    assert_eq!(finished(&first), TransferStatus::Done);
    assert_eq!(finished(&second), TransferStatus::Done);
    assert_eq!(first_rx.status(), TransferStatus::Done);
    assert_eq!(second_rx.take_data().unwrap(), vec![7; 10]);
}

#[test]
fn sending_without_a_connection_is_refused() {
    let mut link = Link::new();
    assert_eq!(
        link.server
            .send(DOWN, single("a", 1), Bytes::new(), Source::bytes(vec![1]))
            .unwrap_err(),
        TransferError::NotConnected
    );
    assert_eq!(
        link.client.reserve(UP).unwrap_err(),
        TransferError::NotConnected
    );
    link.connect();
    link.disconnect();
    assert_eq!(
        link.server
            .send(DOWN, single("a", 1), Bytes::new(), Source::bytes(vec![1]))
            .unwrap_err(),
        TransferError::NotConnected
    );
}

#[test]
fn a_disconnect_fails_every_transfer_and_frees_the_endpoints() {
    let mut link = Link::new();
    link.connect();
    let body = single("big", 1 << 30);
    let sent = link
        .server
        .send(DOWN, body, Bytes::new(), pattern(1 << 30))
        .unwrap();
    let offer = link.client_offer(DOWN);
    let received = offer.token.accept(Sink::Discard, AcceptMode::Lane, None);
    let waiting = send_bytes(&link.server, DOWN2, vec![1; 3]);
    let pending = link.client_offer(DOWN2);
    until(|| (received.progress().done > 0).then_some(())).unwrap();

    link.disconnect();
    for shared in [&sent, &received, &waiting] {
        assert_eq!(
            shared.status(),
            TransferStatus::Failed(TransferError::Disconnected)
        );
    }
    assert!(pending.token.is_cancelled());
    assert_eq!(
        pending
            .token
            .accept(Sink::memory(3), AcceptMode::Lane, None)
            .status(),
        TransferStatus::Failed(TransferError::Disconnected)
    );
    assert!(!link.server.is_busy(DOWN) && !link.client.is_busy(DOWN));
    assert_eq!(
        link.client.last(DOWN).unwrap().status(),
        TransferStatus::Failed(TransferError::Disconnected),
        "last() keeps the failed handle"
    );

    // A new connection starts clean.
    link.connect();
    let again = send_bytes(&link.server, DOWN, vec![2; 4]);
    let received = link
        .client_offer(DOWN)
        .token
        .accept(Sink::memory(4), AcceptMode::Lane, None);
    assert_eq!(finished(&again), TransferStatus::Done);
    assert_eq!(received.take_data().unwrap(), vec![2; 4]);
}

#[test]
fn a_reserved_send_is_busy_until_its_files_are_picked() {
    let mut link = Link::new();
    link.connect();
    let picking = link.client.reserve(UP).unwrap();
    assert!(link.client.is_busy(UP));
    assert_eq!(link.client.reserve(UP).unwrap_err(), TransferError::Busy);
    assert!(Arc::ptr_eq(&link.client.last(UP).unwrap(), &picking));
    link.client.send_reserved(
        &picking,
        single("a", 2),
        Bytes::new(),
        Source::bytes(vec![1, 2]),
    );
    until(|| link.server.take_offer(UP)).unwrap().token.accept(
        Sink::memory(2),
        AcceptMode::Lane,
        None,
    );
    assert_eq!(finished(&picking), TransferStatus::Done);

    // Cancelled while picking: the picked files are never offered.
    let picking = link.client.reserve(UP).unwrap();
    picking.cancel(None);
    assert!(!link.client.is_busy(UP));
    link.client.send_reserved(
        &picking,
        single("a", 2),
        Bytes::new(),
        Source::bytes(vec![1, 2]),
    );
    assert!(!link.client.is_busy(UP));
    std::thread::sleep(Duration::from_millis(50));
    assert!(link.server.take_offer(UP).is_none());
}

#[test]
fn a_server_without_receiver_rejects_offers() {
    let mut link = Link::new();
    link.server.set_receiver(UP, false);
    link.connect();
    let sent = send_bytes(&link.client, UP, vec![1]);
    assert_eq!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Rejected("no receiver".to_string()))
    );
}

/// A receiving engine driven frame by frame, to check what it does with a
/// sender that breaks the protocol.
struct Receiver {
    engine: Engine,
    out: std::sync::mpsc::Receiver<Vec<u8>>,
}

impl Receiver {
    fn new() -> Self {
        let engine = Engine::new(Side::Client, None);
        engine.register(DOWN, Role::Receive, 0);
        let (tx, out) = std::sync::mpsc::channel();
        engine.attach(Arc::new(move |message| {
            let _ = tx.send(message);
        }));
        Self { engine, out }
    }

    fn offer(&self, serial: u32, size: u64) {
        let payload = encode_offer(&single("x", size), &[]).unwrap();
        self.engine.handle_frame(
            DOWN,
            TransferFrame::Offer {
                serial,
                meta_type: 0,
            },
            Bytes::from(payload),
        );
    }

    fn chunk(&self, serial: u32, last: bool, len: usize) {
        self.engine.handle_frame(
            DOWN,
            TransferFrame::Chunk { serial, last },
            Bytes::from(vec![0; len]),
        );
    }

    fn frames(&self) -> Vec<TransferFrame> {
        self.out
            .try_iter()
            .map(|message| decode(Side::Client, &message).1)
            .collect()
    }

    fn accepted(&self, serial: u32, size: u64) -> Arc<TransferShared> {
        self.offer(serial, size);
        let offer = self.engine.take_offer(DOWN).unwrap();
        let shared = offer.token.accept(Sink::Discard, AcceptMode::Lane, None);
        assert_eq!(
            self.frames(),
            [TransferFrame::Accept {
                serial,
                mode: AcceptMode::Lane
            }]
        );
        shared
    }
}

fn protocol_error(shared: &TransferShared) -> bool {
    matches!(
        shared.status(),
        TransferStatus::Failed(TransferError::Protocol(_))
    )
}

#[test]
fn data_beyond_the_credit_fails_the_transfer() {
    let rx = Receiver::new();
    let shared = rx.accepted(3, 10 * FILE_TRANSFER_WINDOW);
    rx.chunk(3, false, FILE_TRANSFER_WINDOW as usize + 1);
    assert!(protocol_error(&shared), "{:?}", shared.status());
    assert!(matches!(
        rx.frames()[..],
        [TransferFrame::Finished {
            serial: 3,
            error: Some(_)
        }]
    ));
}

#[test]
fn credit_is_returned_as_the_sink_takes_data() {
    let rx = Receiver::new();
    let total = 4 * FILE_TRANSFER_WINDOW;
    let shared = rx.accepted(1, total);
    let chunk = FILE_TRANSFER_CHUNK;
    let mut granted = FILE_TRANSFER_WINDOW;
    let mut sent = 0;
    while sent < total {
        let len = (chunk as u64).min(total - sent).min(granted - sent);
        assert!(len > 0, "the sender would stall at {sent}");
        sent += len;
        rx.chunk(1, sent == total, len as usize);
        for frame in rx.frames() {
            match frame {
                TransferFrame::Credit { serial: 1, bytes } => granted += u64::from(bytes),
                TransferFrame::Finished {
                    serial: 1,
                    error: None,
                } => assert_eq!(sent, total),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(granted - sent <= FILE_TRANSFER_WINDOW, "outstanding credit");
    }
    assert_eq!(shared.status(), TransferStatus::Done);
}

#[test]
fn more_data_than_announced_or_a_wrong_last_flag_fail_the_transfer() {
    let rx = Receiver::new();
    let shared = rx.accepted(1, 10);
    rx.chunk(1, true, 11);
    assert!(protocol_error(&shared));

    let rx = Receiver::new();
    let shared = rx.accepted(1, 10);
    rx.chunk(1, true, 5);
    assert!(protocol_error(&shared));

    let rx = Receiver::new();
    let shared = rx.accepted(1, 10);
    rx.chunk(1, false, 10);
    assert!(protocol_error(&shared));
}

#[test]
fn stale_serials_are_ignored() {
    let rx = Receiver::new();
    let shared = rx.accepted(5, 10);
    rx.chunk(4, false, 3);
    rx.engine
        .handle_frame(DOWN, TransferFrame::Cancel { serial: 4 }, Bytes::new());
    assert_eq!(shared.status(), TransferStatus::Pending);
    assert!(rx.frames().is_empty());
    rx.chunk(5, true, 10);
    assert_eq!(shared.status(), TransferStatus::Done);
}

#[test]
fn invalid_or_unexpected_offers_are_rejected() {
    let rx = Receiver::new();
    rx.offer(1, 10);
    // A second offer while one is open.
    rx.offer(2, 10);
    assert_eq!(rx.frames(), [TransferFrame::Reject { serial: 2 }]);

    let rx = Receiver::new();
    let body = OfferBody {
        name: "evil".to_string(),
        files: vec![FileEntry {
            name: "../etc/passwd".to_string(),
            size: 1,
            modified: None,
        }],
    };
    rx.engine.handle_frame(
        DOWN,
        TransferFrame::Offer {
            serial: 1,
            meta_type: 0,
        },
        Bytes::from(encode_offer(&body, &[]).unwrap()),
    );
    assert_eq!(rx.frames(), [TransferFrame::Reject { serial: 1 }]);
    assert!(rx.engine.take_offer(DOWN).is_none());

    let rx = Receiver::new();
    rx.engine.handle_frame(
        DOWN,
        TransferFrame::Offer {
            serial: 1,
            meta_type: 99,
        },
        Bytes::from(encode_offer(&single("x", 1), &[]).unwrap()),
    );
    assert_eq!(rx.frames(), [TransferFrame::Reject { serial: 1 }]);

    let rx = Receiver::new();
    rx.engine.handle_frame(
        999,
        TransferFrame::Offer {
            serial: 1,
            meta_type: 0,
        },
        Bytes::from(encode_offer(&single("x", 1), &[]).unwrap()),
    );
    assert_eq!(rx.frames(), [TransferFrame::Reject { serial: 1 }]);
}

#[test]
fn invalid_sends_are_refused() {
    let mut link = Link::new();
    link.connect();
    let bad = single("../x", 1);
    assert!(matches!(
        link.server
            .send(DOWN, bad, Bytes::new(), Source::bytes(vec![1]))
            .unwrap_err(),
        TransferError::InvalidInput(_)
    ));
    assert!(matches!(
        link.server
            .send(
                DOWN,
                single("a", 1),
                Bytes::from(vec![0; VALUE_MAX_SIZE + 1]),
                Source::bytes(vec![1])
            )
            .unwrap_err(),
        TransferError::Meta(_)
    ));
    assert!(matches!(
        link.server
            .send(UP, single("a", 1), Bytes::new(), Source::bytes(vec![1]))
            .unwrap_err(),
        TransferError::InvalidInput(_)
    ));
    assert!(!link.server.is_busy(DOWN));
}

#[test]
fn a_source_shorter_than_announced_fails_on_both_sides() {
    let mut link = Link::new();
    link.connect();
    let sent = link
        .server
        .send(
            DOWN,
            single("a", 100),
            Bytes::new(),
            Source::bytes(vec![1; 50]),
        )
        .unwrap();
    let received = link
        .client_offer(DOWN)
        .token
        .accept(Sink::memory(100), AcceptMode::Lane, None);
    assert!(matches!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Io(_))
    ));
    assert!(matches!(
        finished(&received),
        TransferStatus::Failed(TransferError::Io(_))
    ));
}

#[cfg(any(feature = "server", feature = "python"))]
#[test]
fn waiting_sends_are_served_in_call_order() {
    let mut link = Link::new();
    link.connect();
    let first = send_bytes(&link.server, DOWN, vec![1]);
    assert_eq!(
        link.server
            .send_wait(
                DOWN,
                single("a", 1),
                Bytes::new(),
                Source::bytes(vec![1]),
                Some(Duration::from_millis(20))
            )
            .unwrap_err(),
        TransferError::Timeout
    );

    let order = Arc::new(Mutex::new(Vec::new()));
    let threads: Vec<_> = (0..3)
        .map(|i| {
            let server = link.server.clone();
            let order = order.clone();
            let thread = std::thread::spawn(move || {
                let shared = server
                    .send_wait(
                        DOWN,
                        single("a", 1),
                        Bytes::new(),
                        Source::bytes(vec![i]),
                        Some(DEADLINE),
                    )
                    .unwrap();
                order.lock().push(i);
                shared
            });
            // Let each thread queue up before the next one.
            std::thread::sleep(Duration::from_millis(30));
            thread
        })
        .collect();
    // Answer the offers as they come.
    let mut offers = 0;
    let client = link.client.clone();
    let answer = std::thread::spawn(move || {
        while offers < 4 {
            if let Some(offer) = client.take_offer(DOWN) {
                offer.token.accept(Sink::memory(1), AcceptMode::Lane, None);
                offers += 1;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    for thread in threads {
        let shared = thread.join().unwrap();
        assert_eq!(finished(&shared), TransferStatus::Done);
    }
    answer.join().unwrap();
    assert_eq!(finished(&first), TransferStatus::Done);
    assert_eq!(*order.lock(), [0, 1, 2]);

    // A disconnect ends a wait.
    let _busy = send_bytes(&link.server, DOWN, vec![1]);
    let server = link.server.clone();
    let waiter = std::thread::spawn(move || {
        server.send_wait(
            DOWN,
            single("a", 1),
            Bytes::new(),
            Source::bytes(vec![1]),
            None,
        )
    });
    std::thread::sleep(Duration::from_millis(30));
    link.disconnect();
    assert_eq!(
        waiter.join().unwrap().unwrap_err(),
        TransferError::NotConnected
    );
}

#[test]
fn removing_the_last_receiver_rejects_a_waiting_offer() {
    let mut link = Link::new();
    link.connect();
    let sent = send_bytes(&link.client, UP, vec![1; 4]);
    until(|| {
        link.server
            .0
            .state
            .lock()
            .endpoints
            .get(&UP)
            .and_then(|ep| ep.current.as_ref())
            .map(|_| ())
    })
    .unwrap();
    link.server.set_receiver(UP, false);
    assert_eq!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Rejected("no receiver".to_string()))
    );
}

#[test]
fn an_offer_token_from_an_earlier_connection_does_not_touch_a_new_offer() {
    let engine = Engine::new(Side::Client, None);
    engine.register(DOWN, Role::Receive, 0);
    let lane: StateLane = Arc::new(|_| {});
    let first = engine.attach(lane.clone());
    let offer = |serial| {
        let payload = encode_offer(&single("x", 4), &[]).unwrap();
        engine.handle_frame(
            DOWN,
            TransferFrame::Offer {
                serial,
                meta_type: 0,
            },
            Bytes::from(payload),
        );
    };
    offer(0);
    let old = engine.take_offer(DOWN).unwrap();
    engine.detach(first);
    engine.attach(lane);
    // The new connection's sender starts its serials again at 0.
    offer(0);
    let new = engine.take_offer(DOWN).unwrap();
    assert!(old.token.is_cancelled());
    assert!(!new.token.is_cancelled());
    let failed = old.token.accept(Sink::memory(4), AcceptMode::Lane, None);
    assert!(failed.is_finished());
    assert!(
        !new.token.is_cancelled(),
        "accepting the old token left the new offer alone"
    );
    let received = new.token.accept(Sink::memory(4), AcceptMode::Lane, None);
    assert_eq!(received.status(), TransferStatus::Pending);
}

#[test]
fn a_disconnect_ends_a_send_whose_files_are_being_picked() {
    let mut link = Link::new();
    link.connect();
    let picking = link.client.reserve(UP).unwrap();
    link.disconnect();
    assert_eq!(
        picking.status(),
        TransferStatus::Failed(TransferError::Disconnected)
    );
    assert!(!link.client.is_busy(UP));
    link.connect();
    link.client.send_reserved(
        &picking,
        single("a", 1),
        Bytes::new(),
        Source::bytes(vec![1]),
    );
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        link.server.take_offer(UP).is_none(),
        "nothing reaches the new connection"
    );
}

#[test]
fn the_next_transfer_waits_until_the_receiver_confirmed_the_last() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let engine = Engine::new(Side::Server, None);
    engine.register(DOWN, Role::Send, 0);
    engine.register(DOWN2, Role::Send, 0);
    let generation = engine.attach(Arc::new(|_| {}));
    let first = send_bytes(&engine, DOWN, vec![1; 10]);
    let second = send_bytes(&engine, DOWN2, vec![2; 10]);
    for endpoint in [DOWN, DOWN2] {
        engine.handle_frame(
            endpoint,
            TransferFrame::Accept {
                serial: 0,
                mode: AcceptMode::Lane,
            },
            Bytes::new(),
        );
    }
    let (data, mut chunks) = mpsc::channel(4);
    let pump = engine.clone();
    runtime.block_on(async {
        let pumping = tokio::spawn(async move { pump.pump(generation, data).await });
        let (endpoint, _, _) = decode(Side::Server, &chunks.recv().await.unwrap());
        assert_eq!(endpoint, DOWN);
        // The second transfer's data waits for the first's confirmation.
        let early = tokio::time::timeout(Duration::from_millis(100), chunks.recv()).await;
        assert!(early.is_err(), "the second transfer started early");
        engine.handle_frame(
            DOWN,
            TransferFrame::Finished {
                serial: 0,
                error: None,
            },
            Bytes::new(),
        );
        let (endpoint, _, _) = decode(Side::Server, &chunks.recv().await.unwrap());
        assert_eq!(endpoint, DOWN2);
        engine.detach(generation);
        let _ = pumping.await;
    });
    assert_eq!(first.status(), TransferStatus::Done);
    assert!(second.is_finished());
}

#[test]
fn offers_beyond_the_size_limits_are_rejected_on_receipt() {
    let rx = Receiver::new();
    let payload = encode_offer(&single("x", 1), &vec![0; VALUE_MAX_SIZE + 1]).unwrap();
    rx.engine.handle_frame(
        DOWN,
        TransferFrame::Offer {
            serial: 1,
            meta_type: 0,
        },
        Bytes::from(payload),
    );
    assert_eq!(rx.frames(), [TransferFrame::Reject { serial: 1 }]);
    assert!(rx.engine.take_offer(DOWN).is_none());

    // Sizes that overflow together.
    let rx = Receiver::new();
    let body = OfferBody {
        name: "set".to_string(),
        files: vec![
            FileEntry {
                name: "a".to_string(),
                size: u64::MAX,
                modified: None,
            },
            FileEntry {
                name: "b".to_string(),
                size: 1,
                modified: None,
            },
        ],
    };
    rx.engine.handle_frame(
        DOWN,
        TransferFrame::Offer {
            serial: 2,
            meta_type: 0,
        },
        Bytes::from(encode_offer(&body, &[]).unwrap()),
    );
    assert_eq!(rx.frames(), [TransferFrame::Reject { serial: 2 }]);

    // A payload over both limits together is refused before it is decoded:
    // this one would not decode.
    let rx = Receiver::new();
    rx.engine.handle_frame(
        DOWN,
        TransferFrame::Offer {
            serial: 3,
            meta_type: 0,
        },
        Bytes::from(vec![0xff; 2 * VALUE_MAX_SIZE + 1]),
    );
    let (_, frame, reason) = decode(Side::Client, &rx.out.try_recv().unwrap());
    assert_eq!(frame, TransferFrame::Reject { serial: 3 });
    assert_eq!(&reason[..], b"the offer is too large");
}

/// `last` is the handle the application was given last, also when the offer
/// behind it had already ended; only a later transfer's handle stays.
#[test]
fn a_handle_for_an_ended_offer_is_the_last_one() {
    let rx = Receiver::new();
    let cancel = |serial| {
        rx.engine
            .handle_frame(DOWN, TransferFrame::Cancel { serial }, Bytes::new());
    };
    let done = rx.accepted(1, 0);
    rx.chunk(1, true, 0);
    assert_eq!(done.status(), TransferStatus::Done);
    assert!(Arc::ptr_eq(&rx.engine.last(DOWN).unwrap(), &done));

    // The sender withdraws the offer while the destination is chosen.
    rx.offer(2, 4);
    let withdrawn = rx.engine.take_offer(DOWN).unwrap();
    cancel(2);
    let failed = withdrawn
        .token
        .accept(Sink::memory(4), AcceptMode::Lane, None);
    assert!(matches!(
        failed.status(),
        TransferStatus::Failed(TransferError::Cancelled { by_peer: true, .. })
    ));
    assert!(Arc::ptr_eq(&rx.engine.last(DOWN).unwrap(), &failed));

    // The same for a destination picked off the calling thread.
    rx.offer(3, 4);
    let picked = rx.engine.take_offer(DOWN).unwrap();
    cancel(3);
    picked.token.expose();
    assert!(Arc::ptr_eq(
        &rx.engine.last(DOWN).unwrap(),
        picked.token.shared()
    ));

    // An offer answered after a later transfer got its handle does not
    // replace it.
    rx.offer(4, 4);
    let late = rx.engine.take_offer(DOWN).unwrap();
    cancel(4);
    rx.frames();
    let newer = rx.accepted(5, 4);
    let stale = late.token.accept(Sink::memory(4), AcceptMode::Lane, None);
    assert!(stale.is_finished());
    assert!(Arc::ptr_eq(&rx.engine.last(DOWN).unwrap(), &newer));
}
