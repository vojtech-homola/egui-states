//! Shared helpers for the `egui_states` benchmarks.
//!
//! Run them with `cargo bench -p egui_states_bench`; see `benches/` for the
//! individual groups.

use std::net::{Ipv4Addr, TcpListener};
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
