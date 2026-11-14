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
    port: u16,
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
            port,
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

#[test]
fn uploads_reach_the_server_receiver() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let source = temp_dir("upload_source");
    let target = temp_dir("upload_target");
    std::fs::write(source.join("a.bin"), data(300_000)).unwrap();
    std::fs::write(source.join("b.bin"), data(7)).unwrap();

    // Without a receiver, offers are rejected.
    let refused = peers.states.inputs.send_bytes(vec![1], "x", 0);
    assert_eq!(
        finished(&refused),
        TransferStatus::Failed(TransferError::Rejected("no receiver".to_string()))
    );

    let dir = target.clone();
    let offers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = offers.clone();
    let receiver = peers.inputs.on_offer(move |offer| {
        seen.lock()
            .unwrap()
            .push((offer.name().to_string(), *offer.meta()));
        if offer.total_size() > Some(1_000_000) {
            offer.reject("too large");
        } else if offer.files().len() == 1 && offer.name() == "mem.bin" {
            offer.accept_memory(1024).unwrap();
        } else {
            offer.accept_dir(&dir).unwrap();
        }
    });

    let uploaded = peers.states.inputs.send_files(
        vec![source.join("a.bin").into(), source.join("b.bin").into()],
        "set",
        5,
    );
    assert_eq!(finished(&uploaded), TransferStatus::Done);
    assert_eq!(listing(&target), ["a.bin", "b.bin"]);
    assert_eq!(std::fs::read(target.join("a.bin")).unwrap(), data(300_000));
    let server_side = wait("server handle", || peers.inputs.last());
    assert_eq!(server_side.status(), TransferStatus::Done);
    assert_eq!(server_side.path(), Some(target.clone()));

    let in_memory = peers
        .states
        .inputs
        .send_bytes(b"small".to_vec(), "mem.bin", 6);
    assert_eq!(finished(&in_memory), TransferStatus::Done);
    let server_side = wait("server handle", || {
        peers.inputs.last().filter(|h| h.name() == "mem.bin")
    });
    server_side.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(server_side.take_data().unwrap(), b"small");

    let too_large = peers
        .states
        .inputs
        .send_bytes(vec![0; 2_000_000], "big.bin", 7);
    assert_eq!(
        finished(&too_large),
        TransferStatus::Failed(TransferError::Rejected("too large".to_string()))
    );
    assert_eq!(
        *offers.lock().unwrap(),
        [
            ("set".to_string(), 5),
            ("mem.bin".to_string(), 6),
            ("big.bin".to_string(), 7)
        ]
    );

    // Client-side failures: a missing file and a closed dialog.
    let missing = peers.states.inputs.send_file(source.join("missing"), 0);
    assert!(matches!(
        finished(&missing),
        TransferStatus::Failed(TransferError::Io(_))
    ));
    let (release, released) = std::sync::mpsc::channel::<()>();
    let picking = peers.states.inputs.send_file_with(
        move || {
            released.recv().unwrap();
            None
        },
        0,
    );
    assert!(peers.states.inputs.is_busy());
    let busy = peers.states.inputs.send_bytes(vec![1], "x", 0);
    assert_eq!(busy.status(), TransferStatus::Failed(TransferError::Busy));
    release.send(()).unwrap();
    assert_eq!(
        finished(&picking),
        TransferStatus::Failed(TransferError::Cancelled {
            by_peer: false,
            reason: None
        })
    );
    wait("free", || (!peers.states.inputs.is_busy()).then_some(()));

    // An async picker on its own thread.
    let path = source.join("b.bin");
    let picked = peers
        .states
        .inputs
        .send_file_with_async(async move { Some(path.into()) }, 8);
    assert_eq!(finished(&picked), TransferStatus::Done);

    // Dropping the receiver rejects offers again.
    drop(receiver);
    let refused = peers.states.inputs.send_bytes(vec![1], "x", 0);
    assert_eq!(
        finished(&refused),
        TransferStatus::Failed(TransferError::Rejected("no receiver".to_string()))
    );
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
}

/// A minimal browser: one HTTP request, the status, headers and body.
fn http(port: u16, method: &str, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(response[..split].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(": ").unwrap();
            (name.to_ascii_lowercase(), value.to_string())
        })
        .collect();
    (status, headers, response[split + 4..].to_vec())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> &'a str {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}

/// Collects the downloads the client would hand to the browser.
fn browser(
    peers: &Peers,
) -> std::sync::mpsc::Receiver<crate::file_transfer::engine::BrowserDownload> {
    let (sender, downloads) = std::sync::mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    peers
        .states
        .files
        .engine
        .set_ticket_hook(std::sync::Arc::new(move |download| {
            let _ = sender.lock().unwrap().send(download);
        }));
    downloads
}

fn path(download: &crate::file_transfer::engine::BrowserDownload) -> String {
    format!(
        "{}{}",
        crate::file_transfer::engine::DOWNLOAD_PATH,
        download.ticket
    )
}

#[test]
fn browser_downloads_stream_over_http() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let downloads = browser(&peers);
    let next = || downloads.recv_timeout(Duration::from_secs(10)).unwrap();

    // One file.
    let payload = data(1_500_000);
    let sent = peers
        .files
        .send_bytes(payload.clone(), "ünï a.bin", &(), false, None)
        .unwrap();
    let received = wait("offer", || peers.states.files.take_offer()).accept_browser(false);
    let download = next();
    assert_eq!(download.name, "ünï a.bin");
    // A HEAD does not use the ticket up.
    assert_eq!(http(peers.port, "HEAD", &path(&download)).0, 405);
    let (status, headers, body) = http(peers.port, "GET", &path(&download));
    assert_eq!(status, 200);
    assert_eq!(body, payload);
    assert_eq!(
        header(&headers, "content-length"),
        payload.len().to_string()
    );
    assert!(
        header(&headers, "content-disposition").contains("filename*=UTF-8''%C3%BCn%C3%AF%20a.bin")
    );
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(received.progress().done, payload.len() as u64);
    // A ticket works once, and unknown paths are not found.
    assert_eq!(http(peers.port, "GET", &path(&download)).0, 404);
    assert_eq!(http(peers.port, "GET", "/other").0, 404);

    // Several files, one download each, folders flattened.
    let source = temp_dir("browser_source");
    std::fs::create_dir_all(source.join("run")).unwrap();
    std::fs::write(source.join("run/a.txt"), b"alpha").unwrap();
    std::fs::write(source.join("run/b.txt"), b"beta").unwrap();
    let paths = [source.join("run/a.txt"), source.join("run/b.txt")];
    let sent = peers
        .files
        .send_files(&paths, "run", Some(&source), &(), false, None)
        .unwrap();
    let received = wait("offer", || peers.states.files.take_offer()).accept_browser(false);
    let first = next();
    assert_eq!(first.name, "run_a.txt");
    assert_eq!(http(peers.port, "GET", &path(&first)).2, b"alpha");
    let second = next();
    assert_eq!(second.name, "run_b.txt");
    assert_eq!(http(peers.port, "GET", &path(&second)).2, b"beta");
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&received), TransferStatus::Done);
    assert_eq!(received.progress().files_done, 2);

    // Several files as one zip.
    let sent = peers
        .files
        .send_files(&paths, "run", Some(&source), &(), false, None)
        .unwrap();
    let received = wait("offer", || peers.states.files.take_offer()).accept_browser(true);
    let bundle = next();
    assert_eq!(bundle.name, "run.zip");
    let (status, headers, body) = http(peers.port, "GET", &path(&bundle));
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/zip");
    assert_eq!(header(&headers, "content-length"), body.len().to_string());
    assert_eq!(&body[..2], b"PK");
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(finished(&received), TransferStatus::Done);
    let zip = source.join("bundle.zip");
    std::fs::write(&zip, &body).unwrap();
    let script = "import sys, zipfile\nz = zipfile.ZipFile(sys.argv[1])\nassert z.testzip() is None\nprint(sorted((i.filename, z.read(i).decode()) for i in z.infolist()))\n";
    if let Ok(output) = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(&zip)
        .output()
    {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "[('run/a.txt', 'alpha'), ('run/b.txt', 'beta')]"
        );
    }
    let _ = std::fs::remove_dir_all(&source);
}

#[test]
fn a_browser_that_stops_cancels_the_download() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let downloads = browser(&peers);
    let sent = peers
        .files
        .send_bytes(vec![0; 64 * 1024 * 1024], "big.bin", &(), false, None)
        .unwrap();
    let received = wait("offer", || peers.states.files.take_offer()).accept_browser(false);
    let download = downloads.recv_timeout(Duration::from_secs(10)).unwrap();
    {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, peers.port)).unwrap();
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: x\r\n\r\n",
            path(&download)
        )
        .unwrap();
        let mut start = [0u8; 4096];
        stream.read_exact(&mut start).unwrap();
        // The browser closes the connection, as a cancel in its download list does.
    }
    let reason = Some("the browser stopped the download".to_string());
    assert_eq!(
        finished(&sent),
        TransferStatus::Failed(TransferError::Cancelled {
            by_peer: true,
            reason: reason.clone()
        })
    );
    assert_eq!(
        finished(&received),
        TransferStatus::Failed(TransferError::Cancelled {
            by_peer: true,
            reason
        })
    );
}

#[test]
fn connect_callbacks_all_run_and_release_the_transfer() {
    if crate::test_support::isolated() {
        return;
    }
    let peers = Peers::connect();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (first, second) = (calls.clone(), calls.clone());
    let a = peers.files.connect(move |_| {
        first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let b = peers.files.connect(move |_| {
        second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let sent = peers
        .files
        .send_bytes(vec![1; 10], "a", &(), false, None)
        .unwrap();
    wait("offer", || peers.states.files.take_offer()).accept_memory(10);
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    wait("callbacks", || {
        (calls.load(std::sync::atomic::Ordering::SeqCst) == 2).then_some(())
    });
    let key = sent.shared.key();
    assert!(
        peers.files.endpoint.events.take(key).is_none(),
        "the event released it"
    );

    // Without callbacks, finished transfers are not kept at all.
    drop((a, b));
    let sent = peers
        .files
        .send_bytes(vec![1; 10], "b", &(), false, None)
        .unwrap();
    wait("offer", || peers.states.files.take_offer()).accept_memory(10);
    sent.wait(Some(Duration::from_secs(10))).unwrap();
    assert!(
        peers
            .files
            .endpoint
            .events
            .take(sent.shared.key())
            .is_none()
    );
}

#[test]
fn a_connect_callback_can_send_at_once() {
    if crate::test_support::isolated() {
        return;
    }
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let server = StateServer::new().unwrap();
    let files = super::Download::<()>::new(&server, "root.files").unwrap();
    let _inputs = super::Upload::<u32>::new(&server, "root.inputs").unwrap();
    server.finalize().unwrap();
    let (sender, results) = std::sync::mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    let _callback = server.on_connect(move |_| {
        let sent = files.send_bytes(vec![1], "hello", &(), false, None);
        let _ = sender.lock().unwrap().send(sent.map(|_| ()));
    });
    server.start(port, Some(Ipv4Addr::LOCALHOST), None).unwrap();
    let (_states, client) =
        ClientBuilder::<ClientStates>::new(egui::Context::default(), port).build();
    client.connect();
    assert_eq!(
        results.recv_timeout(Duration::from_secs(10)).unwrap(),
        Ok(())
    );
    client.disconnect();
    server.stop();
}
