//! State round-trip latency while a file transfer saturates the connection.
//!
//! One round trip: the client sets a `Value`, the server's callback copies it
//! into a `Static`, and the client sees the `Static` change. It is measured
//! idle, during a download, during an upload, and, for comparison, during
//! large `Data` sets, which travel with the state messages.
//!
//! On loopback the kernel drains every buffer at once, so the benchmark also
//! runs through an in-process relay that limits the rate in each direction,
//! like a network link with a queue in front of it.
//!
//! ```sh
//! # In one process: loopback, a 1 Gbit/s and a 100 Mbit/s relay.
//! cargo bench -p egui_states_bench --bench latency
//! # Choose the relay rates in Mbit/s; 0 means no relay.
//! EGUI_LATENCY_RATES=0,1000 cargo bench -p egui_states_bench --bench latency
//! # On two machines: start the server, then the client.
//! cargo bench -p egui_states_bench --bench latency -- serve 0.0.0.0:9300
//! cargo bench -p egui_states_bench --bench latency -- connect 192.168.1.20:9300
//! ```

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use egui_states::bench_support::{accept_discard, discard_uploads, send_pattern, upload_pattern};
use egui_states::server::{self, CallbackHandle, StateServer};
use egui_states::{
    Client, ClientBuilder, ConnectionState, Queue, State, StatesCreator, TransferHandle,
};
use egui_states_bench::{MIB, free_port, wait_until};

/// Round trips measured per case.
const SAMPLES: usize = 300;
/// Larger than any case can send.
const ENDLESS: u64 = 1 << 50;

const START_DOWNLOAD: u32 = 1;
const STOP_DOWNLOAD: u32 = 2;
const START_DATA: u32 = 3;
const STOP_DATA: u32 = 4;

struct LatencyStates {
    value: egui_states::Value<u64, Queue>,
    echo: egui_states::Static<u64>,
    command: egui_states::Signal<u32, Queue>,
    /// Registered so the server's `Data` sets reach a client state.
    _data: egui_states::Data<u8>,
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
    let data = server::Data::<u8>::new(&server, "root.data").expect("data");
    let files = server::Download::<()>::new(&server, "root.files").expect("files");
    let inputs = server::Upload::<()>::new(&server, "root.inputs").expect("inputs");
    server.finalize().expect("finalize");
    discard_uploads(&inputs);
    server.start(port, Some(addr), None).expect("start");

    let echo_callback = value.connect(move |value| {
        let _ = echo.set(value, false);
    });
    let download: Arc<Mutex<Option<TransferHandle>>> = Arc::default();
    let data_running = Arc::new(AtomicBool::new(false));
    let command_callback = command.connect(move |command| match command {
        START_DOWNLOAD => {
            *download.lock().unwrap() = send_pattern(&files, ENDLESS).ok();
        }
        STOP_DOWNLOAD => {
            if let Some(handle) = download.lock().unwrap().take() {
                handle.cancel();
            }
        }
        START_DATA => {
            data_running.store(true, Ordering::Release);
            let (data, running) = (data.clone(), data_running.clone());
            thread::spawn(move || {
                let buffer = vec![0x5a; 16 * MIB];
                while running.load(Ordering::Acquire) {
                    if data.set(&buffer, false).is_err() {
                        break;
                    }
                }
            });
        }
        STOP_DATA => data_running.store(false, Ordering::Release),
        _ => {}
    });
    Server {
        server,
        _callbacks: [echo_callback, command_callback],
    }
}

/// Forwards connections to `target`, limiting each direction to `rate`
/// bytes per second like a link that sends one chunk after another.
fn relay(target: u16, rate: f64) -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("relay bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(client) = incoming else { break };
            let Ok(server) = TcpStream::connect((Ipv4Addr::LOCALHOST, target)) else {
                continue;
            };
            for stream in [&client, &server] {
                let _ = stream.set_nodelay(true);
            }
            let (client_read, server_read) =
                (client.try_clone().unwrap(), server.try_clone().unwrap());
            thread::spawn(move || pipe(client_read, server, rate));
            thread::spawn(move || pipe(server_read, client, rate));
        }
    });
    port
}

fn pipe(mut from: TcpStream, mut to: TcpStream, rate: f64) {
    let mut buffer = vec![0; 16 * 1024];
    let mut link_free = Instant::now();
    loop {
        let len = match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(len) => len,
        };
        // The chunk leaves once the link is free and arrives after its
        // serialization time.
        let start = link_free.max(Instant::now());
        link_free = start + Duration::from_secs_f64(len as f64 / rate);
        if let Some(wait) = link_free.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        if to.write_all(&buffer[..len]).is_err() {
            break;
        }
    }
    let _ = to.shutdown(Shutdown::Write);
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
        self.states.command.set(command);
    }

    /// Accepts the server's download into a sink that drops the data.
    fn accept_download(&self) -> TransferHandle {
        let offer = {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(offer) = self.states.files.take_offer() {
                    break offer;
                }
                assert!(Instant::now() < deadline, "no download offer");
                thread::sleep(Duration::from_millis(1));
            }
        };
        accept_discard(offer)
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
    let download = peer.accept_download();
    running(&download);
    let (samples, moved) = measured(&download, || peer.round_trips());
    report("download", samples, Some(moved));
    peer.command(STOP_DOWNLOAD);
    wait_until("download end", || download.is_finished());

    let upload = upload_pattern(&peer.states.inputs, ENDLESS);
    running(&upload);
    let (samples, moved) = measured(&upload, || peer.round_trips());
    report("upload", samples, Some(moved));
    upload.cancel();
    wait_until("upload end", || upload.is_finished());

    peer.command(START_DATA);
    thread::sleep(Duration::from_millis(500));
    report("data set", peer.round_trips(), None);
    peer.command(STOP_DATA);
    thread::sleep(Duration::from_millis(500));
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
            let rates = std::env::var("EGUI_LATENCY_RATES").unwrap_or_else(|_| "0,1000,100".into());
            for rate in rates
                .split(',')
                .map(|r| r.trim().parse::<f64>().expect("rate in Mbit/s"))
            {
                let port = free_port();
                let server = start_server(port, Ipv4Addr::LOCALHOST);
                let (label, port) = if rate > 0.0 {
                    (
                        format!("relay {rate} Mbit/s"),
                        relay(port, rate * 1e6 / 8.0),
                    )
                } else {
                    ("loopback".to_string(), port)
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
