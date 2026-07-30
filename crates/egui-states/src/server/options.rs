use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use super::ServerError;

pub type ErrorHandler = Arc<dyn Fn(ServerError) + Send + Sync + 'static>;

pub struct ServerOptions {
    pub port: u16,
    pub ip_addr: Option<Ipv4Addr>,
    pub version: Option<u64>,
    pub token: Option<String>,
    pub signal_workers: usize,
    /// How long dropping the server waits for the signal workers to finish the
    /// callback they are running. A worker only observes the shutdown flag
    /// between callbacks, so one that is inside a blocking callback cannot be
    /// waited on unboundedly -- once this elapses it is detached instead.
    pub shutdown_timeout: Duration,
    pub error_handler: Option<ErrorHandler>,
}

impl ServerOptions {
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
