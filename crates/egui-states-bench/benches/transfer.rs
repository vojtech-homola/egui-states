//! End-to-end benchmarks over a loopback connection: a native Rust server and a
//! native client in one process.
//!
//! - `data_set`: one server `Data::set` until the client has published it.
//! - `image_set`: one RGBA server `Image::set` until the client texture changed.
//! - `client_value_latency`: one client `Value::set_signal` until the server
//!   callback has seen it.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use egui_states::server::{self, CallbackHandle, ImageFormat, StateServer};
use egui_states::{Client, ClientBuilder, ConnectionState, Queue, State, StatesCreator};
use egui_states_bench::{MIB, free_port, pattern, wait_until};

struct BenchStates {
    data: egui_states::Data<u8>,
    image: egui_states::Image,
    value: egui_states::Value<u64, Queue>,
}

impl State for BenchStates {
    const NAME: &'static str = "BenchStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            data: c.data("data"),
            image: c.image("image"),
            value: c.value("value", 0),
        }
    }
}

struct Session {
    server: StateServer,
    data: server::Data<u8>,
    image: server::Image,
    received: Arc<AtomicU64>,
    states: BenchStates,
    client: Client,
    _callback: CallbackHandle,
}

impl Session {
    fn connect() -> Self {
        let port = free_port();
        let server = StateServer::new().expect("server");
        let data = server::Data::<u8>::new(&server, "root.data").expect("data");
        let image = server::Image::new(&server, "root.image").expect("image");
        let value = server::Value::new(&server, "root.value", 0_u64, true).expect("value");
        server.finalize().expect("finalize");
        server
            .start(port, Some(Ipv4Addr::LOCALHOST), None)
            .expect("start");

        let received = Arc::new(AtomicU64::new(0));
        let seen = received.clone();
        let callback = value.connect(move |value| seen.store(value, Ordering::Release));

        let (states, client) =
            ClientBuilder::<BenchStates>::new(egui::Context::default(), port).build();
        client.connect();
        wait_until("connection", || {
            server.is_connected() && client.get_state() == ConnectionState::Connected
        });
        // Let the connect synchronization settle.
        std::thread::sleep(Duration::from_millis(200));

        Self {
            server,
            data,
            image,
            received,
            states,
            client,
            _callback: callback,
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.client.disconnect();
        self.server.stop();
    }
}

fn data_set(c: &mut Criterion, session: &Session) {
    let mut group = c.benchmark_group("data_set");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(10));
    let mut marker = 0_u8;
    for mib in [1, 10, 50] {
        let len = mib * MIB;
        let mut buffer = pattern(len);
        group.throughput(Throughput::Bytes(len as u64));
        group.bench_function(BenchmarkId::from_parameter(format!("{mib}MiB")), |b| {
            b.iter_custom(|iterations| {
                let start = Instant::now();
                for _ in 0..iterations {
                    marker = marker.wrapping_add(1);
                    buffer[0] = marker;
                    session.data.set(&buffer, false).expect("set data");
                    wait_until("data", || {
                        session.states.data.read(|data| {
                            data.is_some_and(|data| data.len() == len && data[0] == marker)
                        })
                    });
                }
                start.elapsed()
            })
        });
    }
    group.finish();
}

fn image_set(c: &mut Criterion, session: &Session) {
    let mut group = c.benchmark_group("image_set");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(10));
    for mib in [1, 10, 50] {
        // RGBA pixels: `mib` MiB as a 1024-pixel-wide image, transposed on
        // every other set so the client can tell consecutive images apart.
        let shape = [mib * 256, 1024];
        let pixels = pattern(shape[0] * shape[1] * 4);
        let mut transposed = false;
        group.throughput(Throughput::Bytes(pixels.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(format!("{mib}MiB")), |b| {
            b.iter_custom(|iterations| {
                let start = Instant::now();
                for _ in 0..iterations {
                    transposed = !transposed;
                    let [height, width] = if transposed {
                        [shape[1], shape[0]]
                    } else {
                        shape
                    };
                    session
                        .image
                        .set(&pixels, [height, width], ImageFormat::ColorAlpha, false)
                        .expect("set image");
                    wait_until("image", || {
                        session.states.image.get_size() == Some([width, height])
                    });
                }
                start.elapsed()
            })
        });
    }
    group.finish();
}

fn client_value_latency(c: &mut Criterion, session: &Session) {
    let mut next = session.received.load(Ordering::Acquire);
    c.bench_function("client_value_latency", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            for _ in 0..iterations {
                next += 1;
                session.states.value.set_signal(next);
                wait_until("value", || session.received.load(Ordering::Acquire) == next);
            }
            start.elapsed()
        })
    });
}

fn transfer(c: &mut Criterion) {
    let session = Session::connect();
    data_set(c, &session);
    image_set(c, &session);
    client_value_latency(c, &session);
}

criterion_group!(benches, transfer);
criterion_main!(benches);
