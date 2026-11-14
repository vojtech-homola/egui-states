//! A Rust client and server in one process, transferring over a real socket.

use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use crate::server::{StateServer, TransferError, TransferStatus};
use crate::{Client, ClientBuilder, ConnectionState, State, StatesCreator};

struct ClientStates {
    files: crate::Download,
    inputs: crate::Upload<u32>,
}

impl State for ClientStates {
    const NAME: &'static str = "ClientStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            files: c.download("files"),
            inputs: c.upload("inputs"),
        }
    }
}

struct Peers {
    server: StateServer,
    files: super::Download,
    inputs: super::Upload<u32>,
    states: ClientStates,
    client: Client,
}

fn wait<T>(label: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out: {label}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn finished(handle: &crate::TransferHandle) -> TransferStatus {
    wait("transfer end", || {
        handle.is_finished().then(|| handle.status())
    })
}

impl Peers {
    fn connect() -> Self {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let server = StateServer::new().unwrap();
        let files = super::Download::new(&server, "root.files").unwrap();
        let inputs = super::Upload::new(&server, "root.inputs").unwrap();
        server.finalize().unwrap();
        server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();

        let (states, client) =
            ClientBuilder::<ClientStates>::new(egui::Context::default(), port).build();
        client.connect();
        wait("connection", || {
            (server.is_connected() && client.get_state() == ConnectionState::Connected)
                .then_some(())
        });
        Self {
            server,
            files,
            inputs,
            states,
            client,
        }
    }

    fn reconnect(&self) {
        self.client.disconnect();
        wait("disconnect", || (!self.server.is_connected()).then_some(()));
        wait("client state", || {
            (self.client.get_state() != ConnectionState::Connected).then_some(())
        });
        self.client.connect();
        wait("reconnection", || {
            (self.server.is_connected() && self.client.get_state() == ConnectionState::Connected)
                .then_some(())
        });
    }
}

impl Drop for Peers {
    fn drop(&mut self) {
        self.client.disconnect();
        self.server.stop();
    }
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 253) as u8).collect()
}

#[test]
fn bytes_travel_both_ways() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();

    let payload = data(3 * 1024 * 1024 + 17);
    let sent = peers
        .files
        .send_bytes(payload.clone(), "a.bin", &(), false, None)
        .unwrap();
    let offer = wait("offer", || peers.states.files.take_offer());
    assert_eq!(offer.name(), "a.bin");
    assert_eq!(offer.total_size(), Some(payload.len() as u64));
    let received = offer.accept_memory(u64::MAX);
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(received.take_data().unwrap(), payload);
    assert_eq!(received.progress().files_done, 1);

    // Upload: the server has no receiver yet, so the offer is rejected.
    let rejected = peers.states.inputs.send_bytes(vec![1, 2, 3], "b.bin", 7);
    assert_eq!(
        finished(&rejected),
        TransferStatus::Failed(TransferError::Rejected("no receiver".to_string()))
    );
    crate::bench_support_test::discard(&peers.inputs);
    let uploaded = peers.states.inputs.send_bytes(data(500_000), "c.bin", 9);
    assert_eq!(finished(&uploaded), TransferStatus::Done);
}

#[test]
fn a_busy_endpoint_refuses_or_waits() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let first = peers
        .files
        .send_bytes(vec![1; 10], "a", &(), false, None)
        .unwrap();
    assert_eq!(
        peers
            .files
            .send_bytes(vec![1], "b", &(), false, None)
            .unwrap_err(),
        TransferError::Busy
    );
    assert_eq!(
        peers
            .files
            .send_bytes(vec![1], "b", &(), true, Some(Duration::from_millis(30)))
            .unwrap_err(),
        TransferError::Timeout
    );
    let files = peers.files.clone();
    let waiter = thread::spawn(move || {
        files.send_bytes(vec![2], "c", &(), true, Some(Duration::from_secs(10)))
    });
    wait("offer", || peers.states.files.take_offer()).accept_memory(10);
    assert_eq!(finished(&first), TransferStatus::Done);
    let second = waiter.join().unwrap().unwrap();
    let data = wait("second offer", || peers.states.files.take_offer()).accept_memory(1);
    second.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&data), TransferStatus::Done);
    assert_eq!(data.take_data().unwrap(), vec![2]);

    // A client upload while busy fails at once and leaves last() alone.
    crate::bench_support_test::discard(&peers.inputs);
    let running = peers.states.inputs.send_bytes(vec![0; 10], "x", 1);
    let refused = peers.states.inputs.send_bytes(vec![0; 10], "y", 1);
    assert!(
        refused.status() == TransferStatus::Failed(TransferError::Busy) || running.is_finished(),
        "{:?}",
        refused.status()
    );
    finished(&running);
}

#[test]
fn cancel_and_disconnect_fail_transfers_on_both_sides() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let size = 200 * 1024 * 1024;
    let sent = peers
        .files
        .send_bytes(vec![0; size], "big", &(), false, None)
        .unwrap();
    let received = wait("offer", || peers.states.files.take_offer()).accept_memory(u64::MAX);
    wait("progress", || (received.progress().done > 0).then_some(()));
    received.cancel();
    assert!(matches!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Cancelled { by_peer: true, .. })
    ));

    let sent = peers
        .files
        .send_bytes(vec![0; size], "big", &(), false, None)
        .unwrap();
    let received = wait("offer", || peers.states.files.take_offer()).accept_memory(u64::MAX);
    wait("progress", || (received.progress().done > 0).then_some(()));
    peers.reconnect();
    assert_eq!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Disconnected)
    );
    assert_eq!(
        finished(&received),
        TransferStatus::Failed(TransferError::Disconnected)
    );
    assert_eq!(
        peers.states.files.last().unwrap().status(),
        TransferStatus::Failed(TransferError::Disconnected)
    );

    // The next connection transfers again.
    let again = peers
        .files
        .send_bytes(vec![5; 100], "again", &(), false, None)
        .unwrap();
    let received = wait("offer", || peers.states.files.take_offer()).accept_memory(100);
    again.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(received.take_data().unwrap(), vec![5; 100]);
}

#[test]
fn sending_without_a_client_is_refused() {
    let server = StateServer::new().unwrap();
    let files = super::Download::<()>::new(&server, "root.files").unwrap();
    server.finalize().unwrap();
    assert_eq!(
        files
            .send_bytes(vec![1], "a", &(), false, None)
            .unwrap_err(),
        TransferError::NotConnected
    );
    assert_eq!(
        files
            .send_bytes(vec![1], "a", &(), false, Some(Duration::from_secs(1)))
            .unwrap_err(),
        TransferError::InvalidInput("a timeout needs wait".to_string())
    );
}
