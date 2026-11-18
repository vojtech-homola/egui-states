//! Shared helpers for the `egui_states` benchmarks.
//!
//! Run them with `cargo bench -p egui_states_bench`; see `benches/` for the
//! individual groups.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

/// Returns a free TCP port on the loopback interface.
pub fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("bind a free port")
        .local_addr()
        .expect("read the bound port")
        .port()
}

/// Spins until `condition` holds, panicking after 60 s.
///
/// Spinning instead of sleeping keeps the measured latency free of timer
/// granularity.
pub fn wait_until(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        std::thread::yield_now();
    }
}

/// Deterministic test bytes.
pub fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

/// One mebibyte.
pub const MIB: usize = 1024 * 1024;

/// A simulated network link; both directions behave the same.
#[derive(Clone, Copy)]
pub struct Link {
    /// Bytes per second.
    rate: f64,
    /// The propagation delay in one direction.
    delay: Duration,
}

impl Link {
    /// Parses `Mbit/s[:one-way delay in ms]`; `None` for 0, which means no
    /// relay.
    pub fn parse(spec: &str) -> Option<Self> {
        let (rate, delay) = spec.trim().split_once(':').unwrap_or((spec.trim(), "0"));
        let rate: f64 = rate.parse().expect("rate in Mbit/s");
        let delay: f64 = delay.parse().expect("one-way delay in ms");
        (rate > 0.0).then(|| Self {
            rate: rate * 1e6 / 8.0,
            delay: Duration::from_secs_f64(delay / 1e3),
        })
    }

    /// A description of the link for a report heading.
    pub fn label(&self) -> String {
        format!(
            "relay {} Mbit/s, {} ms each way",
            self.rate * 8.0 / 1e6,
            self.delay.as_secs_f64() * 1e3
        )
    }
}

/// Bytes of the smallest packet on the simulated link.
const PACKET: usize = 1500;
/// Link time of the largest packet. Faster links forward larger packets, as
/// network cards with segmentation offload do, so the relay keeps up with
/// 10 Gbit/s; a packet stays shorter than the delays the benchmark measures.
const PACKET_TIME: Duration = Duration::from_micros(10);
/// Link time of the packets the relay queues in front of the link. What does
/// not fit waits in the socket buffers, like in those of a real sender.
const LINK_QUEUE: Duration = Duration::from_millis(5);

/// Forwards connections to `target` over `link`; returns the port to connect
/// to.
pub fn relay(target: u16, link: Link) -> u16 {
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
            thread::spawn(move || pipe(client_read, server, link));
            thread::spawn(move || pipe(server_read, client, link));
        }
    });
    port
}

/// One direction of the link: schedules every packet read from `from` and
/// hands it to the thread that delivers it into `to`.
fn pipe(mut from: TcpStream, to: TcpStream, link: Link) {
    // Room for the packets in the queue and on the wire; a receiver that
    // stops reading fills it and so holds the sender up.
    let packet_len = PACKET.max((PACKET_TIME.as_secs_f64() * link.rate) as usize);
    let in_transit = (LINK_QUEUE + link.delay).as_secs_f64() * link.rate;
    let (packets, arriving) = sync_channel(in_transit as usize / packet_len + 16);
    thread::spawn(move || deliver(arriving, to));

    let mut buffer = vec![0; 64 * 1024];
    // When the link has sent everything scheduled so far.
    let mut link_free = Instant::now();
    'link: loop {
        let len = match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(len) => len,
        };
        for packet in buffer[..len].chunks(packet_len) {
            // A packet leaves once the link has sent the ones before it and
            // arrives after its serialization time and the delay. The times
            // are absolute, so a late wake-up costs no rate.
            let start = link_free.max(Instant::now());
            link_free = start + Duration::from_secs_f64(packet.len() as f64 / link.rate);
            if packets
                .send((link_free + link.delay, packet.to_vec()))
                .is_err()
            {
                break 'link;
            }
        }
        // Stop reading while the queue is full.
        if let Some(queued) = link_free.checked_duration_since(Instant::now())
            && queued > LINK_QUEUE
        {
            thread::sleep(queued - LINK_QUEUE / 2);
        }
    }
}

/// Writes every packet into `to` at its arrival time.
fn deliver(arriving: Receiver<(Instant, Vec<u8>)>, mut to: TcpStream) {
    // `thread::sleep` wakes tens of microseconds late; the rest is spun.
    const SPIN: Duration = Duration::from_micros(200);
    for (due, packet) in arriving {
        while let Some(wait) = due.checked_duration_since(Instant::now()) {
            if wait > SPIN {
                thread::sleep(wait - SPIN);
            } else {
                std::hint::spin_loop();
            }
        }
        if to.write_all(&packet).is_err() {
            break;
        }
    }
    let _ = to.shutdown(Shutdown::Write);
}
