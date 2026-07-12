use std::net::Ipv4Addr;
use std::sync::Arc;

use super::ServerError;

pub(super) type ErrorHandler = Arc<dyn Fn(ServerError) + Send + Sync + 'static>;

pub struct ServerOptions {
    pub port: u16,
    pub ip_addr: Option<Ipv4Addr>,
    pub version: Option<u64>,
    pub token: Option<String>,
    pub signal_workers: usize,
    pub error_handler: Option<Arc<dyn Fn(ServerError) + Send + Sync + 'static>>,
}

impl ServerOptions {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            ip_addr: None,
            version: None,
            token: None,
            signal_workers: 3,
            error_handler: None,
        }
    }
}
