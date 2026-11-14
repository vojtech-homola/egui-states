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

fn temp_dir(name: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "egui_states_transfer_{name}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn listing(dir: &std::path::Path) -> Vec<String> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn files_are_saved_with_their_names_and_times() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let source = temp_dir("source");
    let target = temp_dir("target");
    std::fs::create_dir_all(source.join("run/frames")).unwrap();
    std::fs::write(source.join("run/a.bin"), data(700_000)).unwrap();
    std::fs::write(source.join("run/frames/0001.raw"), data(5)).unwrap();
    std::fs::write(source.join("run/empty"), b"").unwrap();
    let time = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::File::options()
        .write(true)
        .open(source.join("run/a.bin"))
        .unwrap()
        .set_modified(time)
        .unwrap();

    let reported = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = reported.clone();
    let _callback = peers.files.connect(move |handle| {
        seen.lock()
            .unwrap()
            .push((handle.name().to_string(), handle.status()));
    });

    // One file, replacing an existing one.
    std::fs::write(target.join("saved.bin"), b"old").unwrap();
    let sent = peers
        .files
        .send_file(source.join("run/a.bin"), None, &(), false, None)
        .unwrap();
    let offer = wait("offer", || peers.states.files.take_offer());
    assert_eq!(offer.name(), "a.bin");
    assert_eq!(
        offer.files()[0].modified(),
        Some(time),
        "the offer carries the time"
    );
    let received = offer.accept_file(target.join("saved.bin"));
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(
        std::fs::read(target.join("saved.bin")).unwrap(),
        data(700_000)
    );
    let modified = std::fs::metadata(target.join("saved.bin"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(modified, time);
    assert_eq!(listing(&target), ["saved.bin"], "no temporary file is left");

    // Several files with their folders.
    let paths = [
        source.join("run/a.bin"),
        source.join("run/frames/0001.raw"),
        source.join("run/empty"),
    ];
    let sent = peers
        .files
        .send_files(&paths, "run", Some(&source), &(), false, None)
        .unwrap();
    let offer = wait("offer", || peers.states.files.take_offer());
    assert_eq!(offer.files().len(), 3);
    let received = offer.accept_dir(target.join("copy"));
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(
        listing(&target.join("copy")),
        ["run/a.bin", "run/empty", "run/frames/0001.raw"]
    );
    assert_eq!(received.progress().files_done, 3);
    assert_eq!(
        std::fs::read(target.join("copy/run/frames/0001.raw")).unwrap(),
        data(5)
    );

    // A folder offer cannot go into one file.
    let sent = peers
        .files
        .send_files(&paths, "run", Some(&source), &(), false, None)
        .unwrap();
    let refused = wait("offer", || peers.states.files.take_offer()).accept_file(target.join("x"));
    assert!(matches!(
        refused.status(),
        TransferStatus::Failed(TransferError::Unsupported(_))
    ));
    assert!(matches!(
        sent.wait(Some(Duration::from_secs(10))).unwrap_err(),
        TransferError::Rejected(_)
    ));

    wait("callbacks", || {
        (reported.lock().unwrap().len() == 3).then_some(())
    });
    let reported = reported.lock().unwrap().clone();
    assert_eq!(reported[0], ("a.bin".to_string(), TransferStatus::Done));
    assert_eq!(reported[1], ("run".to_string(), TransferStatus::Done));
    assert!(matches!(
        reported[2].1,
        TransferStatus::Failed(TransferError::Rejected(_))
    ));
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn pickers_run_off_the_calling_thread() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let target = temp_dir("picked");

    let sent = peers
        .files
        .send_bytes(data(1000), "a.bin", &(), false, None)
        .unwrap();
    let offer = wait("offer", || peers.states.files.take_offer());
    let (release, released) = std::sync::mpsc::channel::<()>();
    let path = target.join("picked.bin");
    let handle = offer.accept_file_with(move |offer| {
        assert_eq!(offer.name(), "a.bin");
        released.recv().unwrap();
        Some(path)
    });
    assert_eq!(handle.status(), TransferStatus::Pending);
    assert!(
        peers.states.files.last().is_some(),
        "last() shows the picking transfer"
    );
    release.send(()).unwrap();
    assert_eq!(finished(&handle), TransferStatus::Done);
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(
        std::fs::read(target.join("picked.bin")).unwrap(),
        data(1000)
    );

    // A closed dialog rejects the offer.
    let sent = peers
        .files
        .send_bytes(data(10), "b.bin", &(), false, None)
        .unwrap();
    let handle = wait("offer", || peers.states.files.take_offer()).accept_dir_with(|_| None);
    assert_eq!(
        finished(&handle),
        TransferStatus::Failed(TransferError::Cancelled {
            by_peer: false,
            reason: None
        })
    );
    assert_eq!(
        sent.wait(Some(Duration::from_secs(10))).unwrap_err(),
        TransferError::Rejected("cancelled by user".to_string())
    );
    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn a_cancelled_download_leaves_no_partial_file() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let source = temp_dir("big_source");
    let target = temp_dir("big_target");
    let file = std::fs::File::create(source.join("big.bin")).unwrap();
    file.set_len(512 * 1024 * 1024).unwrap();
    drop(file);

    let sent = peers
        .files
        .send_file(source.join("big.bin"), None, &(), false, None)
        .unwrap();
    let received =
        wait("offer", || peers.states.files.take_offer()).accept_file(target.join("big.bin"));
    wait("progress", || (received.progress().done > 0).then_some(()));
    sent.cancel_with("enough");
    assert_eq!(
        finished(&received),
        TransferStatus::Failed(TransferError::Cancelled {
            by_peer: true,
            reason: Some("enough".to_string())
        })
    );
    wait("cleanup", || listing(&target).is_empty().then_some(()));

    // A file that changes size after the offer fails both sides.
    std::fs::write(source.join("small.bin"), data(100)).unwrap();
    let sent = peers
        .files
        .send_file(source.join("small.bin"), None, &(), false, None)
        .unwrap();
    std::fs::write(source.join("small.bin"), data(50)).unwrap();
    let received =
        wait("offer", || peers.states.files.take_offer()).accept_file(target.join("small.bin"));
    assert!(matches!(
        sent.wait(Some(Duration::from_secs(10))).unwrap_err(),
        TransferError::Io(_)
    ));
    assert!(matches!(
        finished(&received),
        TransferStatus::Failed(TransferError::Io(_))
    ));
    wait("cleanup", || listing(&target).is_empty().then_some(()));

    assert!(matches!(
        peers
            .files
            .send_file(source.join("missing"), None, &(), false, None)
            .unwrap_err(),
        TransferError::Io(_)
    ));
    assert!(matches!(
        peers
            .files
            .send_files(
                &[source.join("small.bin")],
                "x",
                Some(&target),
                &(),
                false,
                None
            )
            .unwrap_err(),
        TransferError::InvalidInput(_)
    ));
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
}
