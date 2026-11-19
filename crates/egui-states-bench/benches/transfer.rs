//! End-to-end benchmarks over a loopback connection: a native Rust server and a
//! native client in one process.
//!
//! - `data_set`: one server `Data::set` until the client has published it.
//! - `image_set`: one RGBA server `Image::set` until the client texture changed.
//! - `image_update`: one RGBA server `Image::update` of a region until the
//!   client has applied it.
//! - `client_value_burst`: ten large client `Value` sets in a row until the
//!   server callback has seen the last.
//! - `client_value_latency`: one client `Value::set_signal` until the server
//!   callback has seen it.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use egui_states::server::{self, CallbackHandle, ImageFormat, StateServer};
use egui_states::{Client, ClientBuilder, ConnectionState, State, StatesCreator};
use egui_states_bench::{MIB, free_port, pattern, wait_until};

struct BenchStates {
    data: egui_states::Data<u8>,
    image: egui_states::Image,
    value: egui_states::Value<u64>,
    blob: egui_states::Value<Vec<u8>>,
    marker: egui_states::Static<u64>,
}

impl State for BenchStates {
    const NAME: &'static str = "BenchStates";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            data: c.data("data"),
            image: c.image("image"),
            value: c.value("value", 0),
            blob: c.value("blob", Vec::new()),
            marker: c.add_static("marker", 0),
        }
    }
}

struct Session {
    server: StateServer,
    data: server::Data<u8>,
    image: server::Image,
    marker: server::Static<u64>,
    received: Arc<AtomicU64>,
    blobs: Arc<AtomicU64>,
    states: BenchStates,
    client: Client,
    _callbacks: [CallbackHandle; 2],
}

impl Session {
    fn connect() -> Self {
        let port = free_port();
        let server = StateServer::new().expect("server");
        let data = server::Data::<u8>::new(&server, "root.data", false).expect("data");
        let image = server::Image::new(&server, "root.image", false).expect("image");
        let value = server::Value::new(&server, "root.value", 0_u64, true).expect("value");
        let blob = server::Value::new(&server, "root.blob", Vec::<u8>::new(), true).expect("blob");
        let marker = server::Static::new(&server, "root.marker", 0_u64).expect("marker");
        server.finalize().expect("finalize");
        server
            .start(port, Some(Ipv4Addr::LOCALHOST), None)
            .expect("start");

        let received = Arc::new(AtomicU64::new(0));
        let seen = received.clone();
        let callback = value.connect(move |value| seen.store(value, Ordering::Release));
        // The first eight bytes of a blob carry its sequence number.
        let blobs = Arc::new(AtomicU64::new(0));
        let seen = blobs.clone();
        let blob_callback = blob.connect(move |blob: Vec<u8>| {
            let number = u64::from_le_bytes(blob[..8].try_into().unwrap());
            seen.store(number, Ordering::Release);
        });

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
            marker,
            received,
            blobs,
            states,
            client,
            _callbacks: [callback, blob_callback],
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

fn image_update(c: &mut Criterion, session: &Session) {
    let mut group = c.benchmark_group("image_update");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(10));
    // A 50 MiB RGBA image whose top rows are replaced.
    let width = 1024;
    let base = pattern(50 * 256 * width * 4);
    session
        .image
        .set(&base, [50 * 256, width], ImageFormat::ColorAlpha, false)
        .expect("set base image");
    wait_until("base image", || {
        session.states.image.get_size() == Some([width, 50 * 256])
    });
    let mut marker = session.states.marker.get();
    for mib in [1, 10, 50] {
        let size = [mib * 256, width];
        let pixels = pattern(size[0] * size[1] * 4);
        group.throughput(Throughput::Bytes(pixels.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(format!("{mib}MiB")), |b| {
            b.iter_custom(|iterations| {
                let start = Instant::now();
                for _ in 0..iterations {
                    session
                        .image
                        .update(
                            &pixels,
                            [0, 0],
                            size,
                            ImageFormat::ColorAlpha,
                            true,
                            None,
                            false,
                        )
                        .expect("update image");
                    // The update's chunks are queued at once and acknowledged
                    // once. A second update elsewhere waits behind it, and the
                    // marker queued after both arrives once the client has
                    // applied the first.
                    session
                        .image
                        .update(
                            &[0; 4],
                            [50 * 256 - 1, width - 1],
                            [1, 1],
                            ImageFormat::ColorAlpha,
                            false,
                            None,
                            false,
                        )
                        .expect("update corner");
                    marker += 1;
                    session.marker.set(marker).expect("set marker");
                    wait_until("image update", || session.states.marker.get() == marker);
                }
                start.elapsed()
            })
        });
    }
    group.finish();
}

fn client_value_burst(c: &mut Criterion, session: &Session) {
    let mut group = c.benchmark_group("client_value_burst");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(10));
    // Ten values just under the 1 MB value limit, sent as fast as possible.
    let mut blob = pattern(900 * 1024);
    let mut next = session.blobs.load(Ordering::Acquire);
    group.throughput(Throughput::Bytes(10 * blob.len() as u64));
    group.bench_function("10x900KiB", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            for _ in 0..iterations {
                for _ in 0..10 {
                    next += 1;
                    blob[..8].copy_from_slice(&next.to_le_bytes());
                    session.states.blob.set_signal(blob.clone());
                }
                wait_until("blobs", || session.blobs.load(Ordering::Acquire) == next);
            }
            start.elapsed()
        })
    });
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
    image_update(c, &session);
    client_value_burst(c, &session);
    client_value_latency(c, &session);
}

criterion_group!(benches, transfer);
criterion_main!(benches);
