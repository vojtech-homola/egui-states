use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use super::ServerError;

/// Thread-safe callback used for asynchronous server errors.
pub type ErrorHandler = Arc<dyn Fn(ServerError) + Send + Sync + 'static>;

/// Configuration used to construct a [`StateServer`](super::StateServer).
pub struct ServerOptions {
    /// TCP port on which the WebSocket server listens.
    pub port: u16,
    /// IPv4 address to bind, or `None` to bind all interfaces.
    pub ip_addr: Option<Ipv4Addr>,
    /// Optional application version required during the client handshake.
    pub version: Option<u64>,
    /// Optional authentication token required during the client handshake.
    pub token: Option<String>,
    /// Number of worker threads that invoke signal callbacks.
    pub signal_workers: usize,
    /// How long dropping the server waits for the signal workers to finish the
    /// callback they are running. A worker only observes the shutdown flag
    /// between callbacks, so one that is inside a blocking callback cannot be
    /// waited on unboundedly -- once this elapses it is detached instead.
    pub shutdown_timeout: Duration,
    /// Handler for asynchronous errors, or `None` to print them to stderr.
    pub error_handler: Option<ErrorHandler>,
}

impl ServerOptions {
    /// Returns default options for `port`.
    ///
    /// The server binds all interfaces, uses three signal workers, and does not
    /// require a version or token.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            ip_addr: None,
            version: None,
            token: None,
            signal_workers: 3,
            shutdown_timeout: Duration::from_secs(2),
            error_handler: None,
        }
    }
}
