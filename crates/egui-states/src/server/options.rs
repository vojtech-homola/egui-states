//! Native server configuration and asynchronous error handling.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::{ClientPolicy, ServerError, TransferProfile};
use crate::server_core::admission::DEFAULT_CONNECT_REQUEST_TIMEOUT;
use crate::server_core::net::heartbeat;

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
    /// How long either side of the connection may hear nothing from the
    /// other before it treats the connection as lost. Default 20 s.
    ///
    /// The server pings the client every quarter of the timeout, and clients
    /// answer pings automatically, so this detects a client that disappeared
    /// without closing the connection (sleeping laptop, lost network), which
    /// would otherwise leave blocking operations waiting for its
    /// acknowledgements. The client learns the timeout when it connects and
    /// ends the connection with `ConnectionLost` when the server goes silent
    /// for that long; in every quarter in which the server sent it nothing
    /// else, the server sends a keep-alive message.
    ///
    /// Nothing else arrives while a large message is in flight, so keep the
    /// timeout above the time the largest message takes over the slowest
    /// expected network. Must be at least 1 second. `None` disables the
    /// heartbeat on both sides, for example while debugging with
    /// breakpoints.
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
    /// How file transfers share the connection with state synchronization:
    /// how much file data may wait ahead of a state change, against how fast
    /// a transfer can go. Default [`TransferProfile::Balanced`]. The client
    /// uses the server's profile.
    pub transfer_profile: TransferProfile,
    /// What the server does with a client that connects while another one
    /// is connected. Default [`ClientPolicy::Replace`]. A callback registered
    /// with [`StateServer::on_connect_request`](super::StateServer::on_connect_request)
    /// decides instead.
    pub client_policy: ClientPolicy,
    /// How long a connect-request callback may take to decide on a client,
    /// counted from the client's handshake. Default 5 s.
    ///
    /// A client still waiting when it passes is refused, which is logged as
    /// an error. The client learns this time and waits as long. Must be
    /// greater than zero.
    pub connect_request_timeout: Duration,
    /// Whether clients connect with TLS, which encrypts the connection.
    /// Default `false`.
    ///
    /// The server then speaks only TLS on its port: `wss://` WebSockets, and
    /// `https://` for the web page and browser downloads. A plain HTTP request
    /// from a browser is redirected to `https://`; other plain connections,
    /// such as a native client without TLS, are refused.
    ///
    /// Without [`Self::tls_cert`], the server generates a self-signed
    /// certificate when it is created. The certificate exists only in memory,
    /// so every new server presents a new one; stopping and starting the same
    /// server keeps it. Native clients pin it by its fingerprint
    /// ([`StateServer::tls_fingerprint`](super::StateServer::tls_fingerprint))
    /// or accept any certificate; browsers show a warning before the page
    /// loads.
    ///
    /// Requires the `tls` feature; without it, a server with any TLS option
    /// set cannot be created.
    pub tls: bool,
    /// A PEM file with the certificate to present instead of a generated one,
    /// followed by any intermediate certificates. Requires [`Self::tls`] and
    /// [`Self::tls_key`].
    ///
    /// Clients that trust the certificate, or the authority that issued it,
    /// connect without a warning, provided they reach the server under a name
    /// or address the certificate lists.
    pub tls_cert: Option<PathBuf>,
    /// The PEM file with the unencrypted private key of [`Self::tls_cert`].
    pub tls_key: Option<PathBuf>,
}

impl ServerOptions {
    /// Returns default server options.
    ///
    /// The server uses three signal workers, does not require an application
    /// version, pings the client every 5 s, disconnecting it after 20 s
    /// without any response, and has no web page. Browser pages may connect
    /// only when the server serves them on an IP address or local name. File
    /// transfers use [`TransferProfile::Balanced`], and a new client replaces
    /// the connected one ([`ClientPolicy::Replace`]). A connect-request
    /// callback may take 5 s. Connections are not encrypted.
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
            heartbeat_timeout: Some(heartbeat::DEFAULT_TIMEOUT),
            web_path: None,
            allowed_origins: Vec::new(),
            transfer_profile: TransferProfile::default(),
            client_policy: ClientPolicy::default(),
            connect_request_timeout: DEFAULT_CONNECT_REQUEST_TIMEOUT,
            tls: false,
            tls_cert: None,
            tls_key: None,
        }
    }
}
