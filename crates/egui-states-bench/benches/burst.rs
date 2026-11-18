//! Bursts of small state messages: how long until the peer has the last one.
//!
//! A burst is a tight loop of sets, so the messages queue faster than the
//! writer sends them and the time depends on how the writer merges queued
//! messages into frames. Every case reports the time from the first set
//! until the peer holds the last value, which is also the latency of that
//! last message.
//!
//! - `static x1`: one server `Static` set repeatedly.
//! - `static x16`: sixteen server `Static`s set in turn.
//! - `value x1`: one server `Value`, which the client acknowledges per set.
//! - `blob`: a server `Static` of the given payload size.
//! - `mixed`: small sets alternating with 16 KiB sets.
//! - `client x1`: one client `Value` set repeatedly.
//! - `round trip`: a client `Value` echoed into a server `Static`, idle and
//!   while the server sets another state 50 000 times a second.
//!
//! ```sh
//! cargo bench -p egui_states_bench --bench burst
//! # Choose the links as `Mbit/s[:one-way delay in ms]`; 0 means no relay.
//! EGUI_BURST_LINKS=0,100:5 cargo bench -p egui_states_bench --bench burst
//! # More bursts per case, for steadier numbers.
//! EGUI_BURST_REPEATS=50 cargo bench -p egui_states_bench --bench burst
//! ```

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use egui_states::server::{self, CallbackHandle, StateServer};
use egui_states::{Client, ClientBuilder, ConnectionState, Queue, State, StatesCreator};
use egui_states_bench::{Link, free_port, relay, wait_until};

/// Bursts measured per case, unless `EGUI_BURST_REPEATS` says otherwise.
const REPEATS: usize = 15;
/// Messages of a burst of small values.
const SMALL_BURST: u64 = 20_000;
/// Payload size and message count of the `blob` cases.
const BLOBS: [(usize, u64); 4] = [
    (1024, 5_000),
    (16 * 1024, 1_000),
    (100 * 1024, 200),
    (400 * 1024, 50),
];
const KIB: usize = 1024;

const NAMES: [&str; 16] = [
    "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "s12", "s13", "s14",
    "s15",
];

struct BurstStates {
    statics: Vec<egui_states::Static<u64>>,
    value: egui_states::Value<u64, Queue>,
    /// Registered so the server's blobs reach a client state.
    _blob: egui_states::Static<String>,
    marker: egui_states::Static<u64>,
    input: egui_states::Value<u64, Queue>,
    ping: egui_states::Value<u64, Queue>,
    echo: egui_states::Static<u64>,
}

impl State for BurstStates {
    const NAME: &'static str = "BurstStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            statics: NAMES.iter().map(|name| c.add_static(name, 0)).collect(),
            value: c.value("value", 0),
            _blob: c.add_static("blob", String::new()),
            marker: c.add_static("marker", 0),
            input: c.value("input", 0),
            ping: c.value("ping", 0),
            echo: c.add_static("echo", 0),
        }
    }
}

struct Session {
    server: StateServer,
    statics: Vec<server::Static<u64>>,
    value: server::Value<u64>,
    blob: server::Static<String>,
    marker: server::Static<u64>,
    /// The last client `input` value the server saw.
    input_seen: Arc<AtomicU64>,
    states: BurstStates,
    client: Client,
    /// The next unused value; every set sends a new one.
    next: u64,
    _callbacks: [CallbackHandle; 2],
}

impl Session {
    fn connect(link: Option<Link>) -> Self {
        let port = free_port();
        let server = StateServer::new().expect("server");
        let statics = NAMES
            .iter()
            .map(|name| {
                server::Static::new(&server, format!("root.{name}"), 0_u64).expect("static")
            })
            .collect();
        let value = server::Value::new(&server, "root.value", 0_u64, false).expect("value");
        let blob = server::Static::new(&server, "root.blob", String::new()).expect("blob");
        let marker = server::Static::new(&server, "root.marker", 0_u64).expect("marker");
        let input = server::Value::new(&server, "root.input", 0_u64, false).expect("input");
        let ping = server::Value::new(&server, "root.ping", 0_u64, true).expect("ping");
        let echo = server::Static::new(&server, "root.echo", 0_u64).expect("echo");
        server.finalize().expect("finalize");
        server
            .start(port, Some(Ipv4Addr::LOCALHOST), None)
            .expect("start");

        let input_seen = Arc::new(AtomicU64::new(0));
        let seen = input_seen.clone();
        let input_callback = input.connect(move |value| seen.store(value, Ordering::Release));
        let echo_callback = ping.connect(move |value| {
            let _ = echo.set(value, false);
        });

        let port = link.map_or(port, |link| relay(port, link));
        let (states, client) =
            ClientBuilder::<BurstStates>::new(egui::Context::default(), port).build();
        client.connect();
        wait_until("connection", || {
            server.is_connected() && client.get_state() == ConnectionState::Connected
        });
        // Let the connect synchronization settle.
        thread::sleep(Duration::from_millis(200));

        Self {
            server,
            statics,
            value,
            blob,
            marker,
            input_seen,
            states,
            client,
            next: 0,
            _callbacks: [input_callback, echo_callback],
        }
    }

    /// Reserves `count` new values and returns the first.
    fn reserve(&mut self, count: u64) -> u64 {
        let first = self.next + 1;
        self.next += count;
        first
    }

    fn statics(&mut self, states: usize) -> Duration {
        let first = self.reserve(SMALL_BURST);
        let last = first + SMALL_BURST - 1;
        let last_state = ((SMALL_BURST - 1) % states as u64) as usize;
        let start = Instant::now();
        for i in 0..SMALL_BURST {
            let _ = self.statics[(i % states as u64) as usize].set(first + i, false);
        }
        wait_until("static burst", || {
            self.states.statics[last_state].get() == last
        });
        start.elapsed()
    }

    fn value(&mut self) -> Duration {
        let first = self.reserve(SMALL_BURST);
        let last = first + SMALL_BURST - 1;
        let start = Instant::now();
        for i in 0..SMALL_BURST {
            let _ = self.value.set(first + i, false);
        }
        wait_until("value burst", || self.states.value.get() == last);
        start.elapsed()
    }

    /// `count` blobs of `size` bytes; with `mixed`, a small set after each.
    fn blobs(&mut self, size: usize, count: u64, mixed: bool) -> Duration {
        // A string is serialized as one block, so the payload costs a copy.
        let payload = "z".repeat(size);
        let first = self.reserve(count + 1);
        let last = first + count;
        let start = Instant::now();
        for i in 0..count {
            let _ = self.blob.set(payload.clone(), false);
            if mixed {
                let _ = self.statics[0].set(first + i, false);
            }
        }
        let _ = self.marker.set(last, false);
        wait_until("blob burst", || self.states.marker.get() == last);
        start.elapsed()
    }

    fn client_values(&mut self) -> Duration {
        let first = self.reserve(SMALL_BURST);
        let last = first + SMALL_BURST - 1;
        let start = Instant::now();
        for i in 0..SMALL_BURST {
            self.states.input.set_signal(first + i);
        }
        wait_until("client burst", || {
            self.input_seen.load(Ordering::Acquire) == last
        });
        start.elapsed()
    }

    fn round_trip(&mut self) -> Duration {
        let next = self.reserve(1);
        let start = Instant::now();
        self.states.ping.set_signal(next);
        wait_until("echo", || self.states.echo.get() == next);
        let elapsed = start.elapsed();
        thread::sleep(Duration::from_millis(1));
        elapsed
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.client.disconnect();
        self.server.stop();
    }
}

fn report(label: &str, mut samples: Vec<Duration>, messages: Option<u64>) {
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let median = samples[samples.len() / 2];
    let rate = messages
        .map(|count| format!("  {:9.0} msg/s", count as f64 / median.as_secs_f64()))
        .unwrap_or_default();
    println!(
        "{label:<18} median {:9.3} ms  min {:9.3} ms  max {:9.3} ms{rate}",
        ms(median),
        ms(samples[0]),
        ms(*samples.last().unwrap()),
    );
}

fn repeat(mut run: impl FnMut() -> Duration) -> Vec<Duration> {
    let repeats = std::env::var("EGUI_BURST_REPEATS")
        .ok()
        .and_then(|repeats| repeats.parse().ok())
        .unwrap_or(REPEATS);
    (0..repeats)
        .map(|_| {
            let elapsed = run();
            // Let acknowledgements and buffers drain before the next burst.
            thread::sleep(Duration::from_millis(20));
            elapsed
        })
        .collect()
}

fn run_cases(session: &mut Session) {
    report(
        "static x1",
        repeat(|| session.statics(1)),
        Some(SMALL_BURST),
    );
    report(
        "static x16",
        repeat(|| session.statics(16)),
        Some(SMALL_BURST),
    );
    report("value x1", repeat(|| session.value()), Some(SMALL_BURST));
    for (size, count) in BLOBS {
        report(
            &format!("blob {} KiB", size / KIB),
            repeat(|| session.blobs(size, count, false)),
            Some(count),
        );
    }
    report(
        "mixed 16 KiB",
        repeat(|| session.blobs(16 * KIB, 500, true)),
        Some(1000),
    );
    report(
        "client x1",
        repeat(|| session.client_values()),
        Some(SMALL_BURST),
    );

    let samples = (0..300).map(|_| session.round_trip()).collect();
    report("round trip idle", samples, None);

    // A steady stream of small sets at a fixed rate on another state.
    let running = Arc::new(AtomicBool::new(true));
    let stream = {
        let (running, state) = (running.clone(), session.statics[1].clone());
        thread::spawn(move || {
            const PERIOD: Duration = Duration::from_micros(20);
            let mut due = Instant::now();
            let mut value = 0;
            while running.load(Ordering::Acquire) {
                while Instant::now() < due {
                    std::hint::spin_loop();
                }
                due += PERIOD;
                value += 1;
                let _ = state.set(value, false);
            }
        })
    };
    thread::sleep(Duration::from_millis(100));
    let samples = (0..300).map(|_| session.round_trip()).collect();
    report("round trip stream", samples, None);
    running.store(false, Ordering::Release);
    stream.join().unwrap();
}

fn main() {
    let links = std::env::var("EGUI_BURST_LINKS").unwrap_or_else(|_| "0,1000:0.1,100:5".into());
    for link in links.split(',').map(Link::parse) {
        let label = link.map_or("loopback".to_string(), |link| link.label());
        let mut session = Session::connect(link);
        println!("== {label}");
        run_cases(&mut session);
    }
}
