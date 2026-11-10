//! Native server configuration and asynchronous error handling.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::ServerError;
use crate::server_core::heartbeat;

/// Thread-safe callback used for asynchronous server errors.
///
/// It runs on a signal-worker thread. Panics from the handler are caught so
/// they cannot unwind out of that worker.
pub type ErrorHandler = Arc<dyn Fn(ServerError) + Send + Sync + 'static>;

/// Configuration used to construct a [`StateServer`](super::StateServer).
pub struct ServerOptions {
    /// Optional application version required during the client handshake.
    pub version: Option<u64>,
    /// Number of worker threads that invoke signal callbacks.
    ///
    /// Callbacks for different state ids may run concurrently. A slow callback
    /// occupies one worker until it returns.
    pub signal_workers: usize,
    /// How long dropping the server waits for the signal workers to finish the
    /// callback they are running. A worker only observes the shutdown flag
    /// between callbacks, so one that is inside a blocking callback cannot be
    /// waited on unboundedly -- once this elapses it is detached instead. This
    /// also bounds how long dropping the final [`StateServer`](super::StateServer)
    /// handle waits for callback workers.
    pub shutdown_timeout: Duration,
    /// Handler for asynchronous errors, or `None` to print them to stderr.
    pub error_handler: Option<ErrorHandler>,
    /// How often the server pings the connected client. Default 5 s.
    ///
    /// Must be greater than zero. Ignored when the heartbeat is disabled.
    pub heartbeat_interval: Duration,
    /// How long the connected client may send nothing before the server
    /// treats the connection as lost and disconnects it. Default 20 s.
    ///
    /// Clients answer pings automatically, so this detects clients that
    /// disappeared without closing the connection (sleeping laptop, lost
    /// network), which would otherwise leave blocking operations waiting for
    /// their acknowledgements. A client stays silent while it receives a large
    /// transfer, so keep the timeout above the time the largest message takes
    /// over the slowest expected network. Must be greater than
    /// [`Self::heartbeat_interval`]. `None` disables the heartbeat.
    pub heartbeat_timeout: Option<Duration>,
    /// Directory of a static web page, such as a built WASM client, that
    /// [`StateServer::start_web`](super::StateServer::start_web) serves on the
    /// server port. `None` (the default) means no page can be served.
    ///
    /// A relative path is resolved against the current directory when the
    /// server is created. The directory is read only by `start_web`, so it
    /// need not exist before then.
    pub web_path: Option<PathBuf>,
    /// Origins of other web pages that may connect, as `scheme://host[:port]`
    /// patterns.
    ///
    /// Browsers let any page open a WebSocket to the server, but state the
    /// page's origin, which the page cannot change. By default only pages
    /// served by the server itself (with [`StateServer::start_web`](super::StateServer::start_web)
    /// or through a proxy that sets `X-Forwarded-Host`, such as `trunk serve`)
    /// on an IP address or a local network name may connect: a name without a
    /// dot (`localhost`, `pc-a`) or ending in `.local`, `.home.arpa`,
    /// `.internal`, or `.localhost`, none of which an internet site can have.
    /// Others are refused before the handshake. Native clients send no origin
    /// and are not affected.
    ///
    /// `*` matches any scheme (`*://host`), any part of the host
    /// (`https://*.example.com`), or any port (`http://myserver.corp.example:*`,
    /// which allows the page opened by that name). Without a port, the
    /// scheme's default port (80 or 443) is meant, as in browsers. An entry
    /// of just `*` allows every page, including local files; any site open in
    /// the browser can then connect, so set a token as well.
    pub allowed_origins: Vec<String>,
}

impl ServerOptions {
    /// Returns default server options.
    ///
    /// The server uses three signal workers, does not require an application
    /// version, pings the client every 5 s, disconnecting it after 20 s
    /// without any response, and has no web page. Browser pages may connect
    /// only when the server serves them on an IP address or local name.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            version: None,
            signal_workers: 3,
            shutdown_timeout: Duration::from_secs(2),
            error_handler: None,
            heartbeat_interval: heartbeat::DEFAULT_INTERVAL,
            heartbeat_timeout: Some(heartbeat::DEFAULT_TIMEOUT),
            web_path: None,
            allowed_origins: Vec::new(),
        }
    }
}
