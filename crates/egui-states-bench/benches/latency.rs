//! State round-trip latency while a file transfer saturates the connection.
//!
//! One round trip: the client sets a `Value`, the server's callback copies it
//! into a `Static`, and the client sees the `Static` change. It is measured
//! idle, during a download into memory and one into a file, during an
//! upload, and during large `Data` sets: of a state with normal priority,
//! which travel with the state messages, and of one with low priority, which
//! use a lane of their own. The file download goes through the sink worker, which
//! writes in batches and returns credit per batch; the others consume every
//! chunk as it arrives.
//!
//! On loopback the kernel drains every buffer at once, so the benchmark also
//! runs through an in-process relay that behaves like a network link: each
//! direction carries a limited rate packet by packet, with a queue in front
//! of it, and every packet arrives a propagation delay later. The relay does
//! not lose or reorder packets.
//!
//! ```sh
//! # In one process: loopback, a 10 Gbit/s and a 1 Gbit/s LAN, and a
//! # 100 Mbit/s WAN.
//! cargo bench -p egui_states_bench --bench latency
//! # Choose the links as `Mbit/s[:one-way delay in ms]`; 0 means no relay.
//! EGUI_LATENCY_LINKS=0,1000:0.1,20:25 cargo bench -p egui_states_bench --bench latency
//! # On two machines: start the server, then the client.
//! cargo bench -p egui_states_bench --bench latency -- serve 0.0.0.0:9300
//! cargo bench -p egui_states_bench --bench latency -- connect 192.168.1.20:9300
//! ```

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use egui_states::bench_support::{accept_discard, discard_uploads, send_pattern, upload_pattern};
use egui_states::server::{self, CallbackHandle, StateServer};
use egui_states::{
    Client, ClientBuilder, ConnectionState, State, StatesCreator, TransferHandle, TransferOffer,
};
use egui_states_bench::{Link, MIB, free_port, relay, wait_until};

/// Round trips measured per case.
const SAMPLES: usize = 300;
/// Larger than any case can send.
const ENDLESS: u64 = 1 << 50;

const START_DOWNLOAD: u32 = 1;
const STOP_DOWNLOAD: u32 = 2;
const START_DATA: u32 = 3;
const STOP_DATA: u32 = 4;
const START_LOW_DATA: u32 = 5;

struct LatencyStates {
    value: egui_states::Value<u64>,
    echo: egui_states::Static<u64>,
    command: egui_states::Signal<u32>,
    /// Registered so the server's `Data` sets reach a client state.
    _data: egui_states::Data<u8>,
    _low_data: egui_states::Data<u8>,
    files: egui_states::Download,
    inputs: egui_states::Upload,
}

impl State for LatencyStates {
    const NAME: &'static str = "LatencyStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            value: c.value("value", 0),
            echo: c.add_static("echo", 0),
            command: c.signal("command"),
            _data: c.data("data"),
            _low_data: c.low_priority(|c| c.data("low_data")),
            files: c.download("files"),
            inputs: c.upload("inputs"),
        }
    }
}

/// The server half: echoes values and starts or stops the server-side load.
struct Server {
    server: StateServer,
    _callbacks: [CallbackHandle; 2],
}

fn start_server(port: u16, addr: Ipv4Addr) -> Server {
    let server = StateServer::new().expect("server");
    let value = server::Value::new(&server, "root.value", 0_u64, true).expect("value");
    let echo = server::Static::new(&server, "root.echo", 0_u64).expect("echo");
    let command = server::Signal::<u32>::new(&server, "root.command", true).expect("command");
    let data = server::Data::<u8>::new(&server, "root.data", false).expect("data");
    let low_data = server::Data::<u8>::new(&server, "root.low_data", true).expect("low data");
    let files = server::Download::<()>::new(&server, "root.files").expect("files");
    let inputs = server::Upload::<()>::new(&server, "root.inputs").expect("inputs");
    server.finalize().expect("finalize");
    discard_uploads(&inputs);
    server.start(port, Some(addr), None).expect("start");

    let echo_callback = value.connect(move |value| {
        let _ = echo.set(value);
    });
    let download: Arc<Mutex<Option<TransferHandle>>> = Arc::default();
    let data_running = Arc::new(AtomicBool::new(false));
    // The producer of the current `Data` case. Stopping joins it, so the next
    // case does not start while its last set still occupies the connection.
    let producer: Arc<Mutex<Option<thread::JoinHandle<()>>>> = Arc::default();
    // Bytes the current case has set, and since when.
    let set_bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let started = Arc::new(Mutex::new(Instant::now()));
    let command_callback = command.connect(move |command| match command {
        START_DOWNLOAD => {
            *download.lock().unwrap() = send_pattern(&files, ENDLESS).ok();
        }
        STOP_DOWNLOAD => {
            if let Some(handle) = download.lock().unwrap().take() {
                handle.cancel();
            }
        }
        START_DATA | START_LOW_DATA => {
            data_running.store(true, Ordering::Release);
            let data = match command {
                START_DATA => data.clone(),
                _ => low_data.clone(),
            };
            let running = data_running.clone();
            let bytes = set_bytes.clone();
            bytes.store(0, Ordering::Release);
            *started.lock().unwrap() = Instant::now();
            *producer.lock().unwrap() = Some(thread::spawn(move || {
                let buffer = vec![0x5a; 16 * MIB];
                while running.load(Ordering::Acquire) {
                    if data.set(&buffer, false).is_err() {
                        break;
                    }
                    bytes.fetch_add(buffer.len() as u64, Ordering::AcqRel);
                }
            }));
        }
        STOP_DATA => {
            data_running.store(false, Ordering::Release);
            if let Some(producer) = producer.lock().unwrap().take() {
                let _ = producer.join();
            }
            // Printed by the server, so also in a two-machine run.
            let rate = set_bytes.load(Ordering::Acquire) as f64
                / started.lock().unwrap().elapsed().as_secs_f64()
                / 1e6;
            println!("{:<10} data sets {rate:7.1} MB/s", "");
        }
        _ => {}
    });
    Server {
        server,
        _callbacks: [echo_callback, command_callback],
    }
}

struct Peer {
    states: LatencyStates,
    client: Client,
    next: u64,
}

fn connect_client(host: &str, port: u16) -> Peer {
    let (states, client) = ClientBuilder::<LatencyStates>::new(egui::Context::default(), port)
        .host(host)
        .build();
    client.connect();
    wait_until("connection", || {
        client.get_state() == ConnectionState::Connected
    });
    thread::sleep(Duration::from_millis(200));
    Peer {
        states,
        client,
        next: 0,
    }
}

impl Peer {
    fn round_trips(&mut self) -> Vec<Duration> {
        (0..SAMPLES)
            .map(|_| {
                self.next += 1;
                let next = self.next;
                let start = Instant::now();
                self.states.value.set_signal(next);
                wait_until("echo", || self.states.echo.get() == next);
                let elapsed = start.elapsed();
                thread::sleep(Duration::from_millis(2));
                elapsed
            })
            .collect()
    }

    fn command(&self, command: u32) {
        self.states.command.emit(command);
    }

    /// Waits for the server's download offer.
    fn download_offer(&self) -> TransferOffer<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(offer) = self.states.files.take_offer() {
                break offer;
            }
            assert!(Instant::now() < deadline, "no download offer");
            thread::sleep(Duration::from_millis(1));
        }
    }
}

fn report(label: &str, mut samples: Vec<Duration>, transferred: Option<(u64, Duration)>) {
    samples.sort();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let rate = transferred
        .map(|(bytes, time)| {
            format!(
                "  transfer {:7.1} MB/s",
                bytes as f64 / time.as_secs_f64() / 1e6
            )
        })
        .unwrap_or_default();
    println!(
        "{label:<10} median {:8.3} ms  p99 {:8.3} ms  max {:8.3} ms{rate}",
        ms(at(0.5)),
        ms(at(0.99)),
        ms(*samples.last().unwrap()),
    );
}

fn running(handle: &TransferHandle) {
    wait_until("transfer start", || handle.progress().done > 0);
}

fn measured(
    handle: &TransferHandle,
    run: impl FnOnce() -> Vec<Duration>,
) -> (Vec<Duration>, (u64, Duration)) {
    let (start_bytes, start) = (handle.progress().done, Instant::now());
    let samples = run();
    (
        samples,
        (handle.progress().done - start_bytes, start.elapsed()),
    )
}

fn run_cases(peer: &mut Peer) {
    report("idle", peer.round_trips(), None);

    peer.command(START_DOWNLOAD);
    let download = accept_discard(peer.download_offer());
    running(&download);
    let (samples, moved) = measured(&download, || peer.round_trips());
    report("download", samples, Some(moved));
    peer.command(STOP_DOWNLOAD);
    wait_until("download end", || download.is_finished());

    // Under the target folder: the temporary folder may live in memory. The
    // cancelled download removes its partial file.
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("latency-download.bin");
    peer.command(START_DOWNLOAD);
    let download = peer.download_offer().accept_file(&path);
    running(&download);
    let (samples, moved) = measured(&download, || peer.round_trips());
    report("to file", samples, Some(moved));
    peer.command(STOP_DOWNLOAD);
    wait_until("file download end", || download.is_finished());

    let upload = upload_pattern(&peer.states.inputs, ENDLESS);
    running(&upload);
    let (samples, moved) = measured(&upload, || peer.round_trips());
    report("upload", samples, Some(moved));
    upload.cancel();
    wait_until("upload end", || upload.is_finished());

    for (label, command) in [("data set", START_DATA), ("low data", START_LOW_DATA)] {
        peer.command(command);
        thread::sleep(Duration::from_millis(500));
        report(label, peer.round_trips(), None);
        // Stopping waits for the producer's last set, up to a few seconds
        // on a slow link.
        peer.command(STOP_DATA);
        thread::sleep(Duration::from_secs(3));
    }
}

fn main() {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with("--"))
        .collect();
    match args.as_slice() {
        [mode, addr] if mode == "serve" => {
            let addr: std::net::SocketAddrV4 = addr.parse().expect("serve ADDR:PORT");
            let _server = start_server(addr.port(), *addr.ip());
            println!("serving on {addr}; stop with Ctrl+C");
            loop {
                thread::park();
            }
        }
        [mode, addr] if mode == "connect" => {
            let (host, port) = addr.rsplit_once(':').expect("connect HOST:PORT");
            let mut peer = connect_client(host, port.parse().expect("port"));
            println!("== remote server {addr}");
            run_cases(&mut peer);
            peer.client.disconnect();
        }
        _ => {
            let links = std::env::var("EGUI_LATENCY_LINKS")
                .unwrap_or_else(|_| "0,10000:0.05,1000:0.1,100:10".into());
            for link in links.split(',').map(Link::parse) {
                let port = free_port();
                let server = start_server(port, Ipv4Addr::LOCALHOST);
                let (label, port) = match link {
                    Some(link) => (link.label(), relay(port, link)),
                    None => ("loopback".to_string(), port),
                };
                let mut peer = connect_client("127.0.0.1", port);
                println!("== {label}");
                run_cases(&mut peer);
                peer.client.disconnect();
                server.server.stop();
            }
        }
    }
}
