//! Failure containment for tests of potentially blocking native operations.

pub(crate) fn isolated() -> bool {
    let name = std::thread::current().name().unwrap().to_owned();
    if std::env::var("EGUI_TEST_CHILD").as_deref() == Ok(&name) {
        std::fs::File::create(std::env::var_os("EGUI_TEST_MARKER").unwrap()).unwrap();
        return false;
    }
    let base = format!(
        "egui-test-{}-{}",
        std::process::id(),
        name.replace(':', "_")
    );
    let path = std::env::temp_dir().join(format!("{base}.log"));
    let marker = std::env::temp_dir().join(format!("{base}.ran"));
    let _ = std::fs::remove_file(&marker);
    let log = std::fs::File::create(&path).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env("EGUI_TEST_CHILD", &name)
        .env("EGUI_TEST_MARKER", &marker)
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let output = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(path);
    assert!(
        status.is_some_and(|s| s.success()),
        "{name}: child failed or exceeded 25s\n{output}"
    );
    // A renamed or moved test matches nothing and would otherwise pass silently.
    // The child marks that it reached this test because its output can
    // interleave with prints from background threads.
    assert!(
        std::fs::remove_file(&marker).is_ok(),
        "{name}: child did not run this test\n{output}"
    );
    true
}

/// Counts the threads of this process started by native clients.
#[cfg(all(feature = "client", target_os = "linux"))]
pub(crate) fn client_thread_count() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.starts_with("Client"))
        .count()
}

#[cfg(feature = "client")]
pub(crate) fn ack(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<
        Option<crate::client::messages::ChannelMessage>,
    >,
    id: u64,
) {
    assert!(
        matches!(
            receiver.try_recv(),
            Ok(Some(
                crate::client::messages::ChannelMessage::Ack(actual)
                    | crate::client::messages::ChannelMessage::AckKey(actual, _)
            )) if actual == id
        ),
        "expected ACK for {id}"
    );
    assert!(receiver.try_recv().is_err(), "unexpected extra message");
}

#[cfg(feature = "client")]
pub(crate) fn texture_updates(
    context: &egui::Context,
) -> std::collections::HashMap<egui::TextureId, Vec<egui::epaint::ImageDelta>> {
    let mut delta = context.tex_manager().write().take_delta();
    let updates = delta
        .set
        .drain()
        .map(|(id, changes)| (id, changes.into_vec()))
        .collect();
    // Headless tests consume the deltas themselves instead of forwarding them to a painter.
    delta.clear();
    updates
}

/// Sends `request` over TLS to the server on `port`, accepting any
/// certificate, and reads the response until the server closes the
/// connection, which it must do with a TLS close notification.
#[cfg(all(
    feature = "client",
    feature = "tls",
    any(feature = "server", feature = "python")
))]
pub(crate) fn https(port: u16, request: &[u8]) -> Vec<u8> {
    use std::io::{Read, Write};

    let mut stream = tls_connect(port);
    stream.write_all(request).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

/// Opens a TLS connection to the server on `port`, accepting any
/// certificate. The handshake completes with the first read or write.
#[cfg(all(
    feature = "client",
    feature = "tls",
    any(feature = "server", feature = "python")
))]
pub(crate) fn tls_connect(
    port: u16,
) -> tokio_rustls::rustls::StreamOwned<tokio_rustls::rustls::ClientConnection, std::net::TcpStream>
{
    use tokio_rustls::rustls::{ClientConnection, StreamOwned};

    let (config, name) =
        crate::client::net::tls::client_config(&crate::Tls::AcceptAny, "localhost").unwrap();
    let tcp = std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    StreamOwned::new(ClientConnection::new(config, name).unwrap(), tcp)
}

/// Returns the messages the server logs at `level` from now on, through a
/// `log` logger installed for the test process.
///
/// The logger is shared by every test in the process, so a test that checks
/// what is logged runs [`isolated`].
#[cfg(all(feature = "client", any(feature = "server", feature = "python")))]
pub(crate) fn server_logs(level: log::Level) -> std::sync::mpsc::Receiver<String> {
    use std::sync::Mutex;
    use std::sync::mpsc::{Sender, channel};

    struct Capture(Mutex<Vec<(log::Level, Sender<String>)>>);

    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.target() == "egui_states"
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                for (level, sender) in self.0.lock().unwrap().iter() {
                    if *level == record.level() {
                        let _ = sender.send(record.args().to_string());
                    }
                }
            }
        }

        fn flush(&self) {}
    }

    static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));
    if log::set_logger(&CAPTURE).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
    let (sender, receiver) = channel();
    CAPTURE.0.lock().unwrap().push((level, sender));
    receiver
}
