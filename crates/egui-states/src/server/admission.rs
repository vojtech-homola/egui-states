//! The client asking to connect, as the connect-request callback sees it.

use std::sync::Arc;

use crate::server_core::admission::Request;

/// A client that passed the handshake and asks to connect.
///
/// The callback registered with
/// [`StateServer::on_connect_request`](super::StateServer::on_connect_request)
/// receives it and decides whether the client may connect.
#[derive(Clone)]
pub struct ConnectRequest(pub(super) Arc<Request>);

impl ConnectRequest {
    /// Returns the client's address, as `ip:port`.
    pub fn address(&self) -> &str {
        self.0.address()
    }

    /// Returns whether the server still waits for the answer.
    ///
    /// It turns `false` once the request was answered, when the client
    /// closes its connection, when
    /// [`ServerOptions::connect_request_timeout`](super::ServerOptions::connect_request_timeout)
    /// passes, or when the server stops. An answer given after that is
    /// ignored, so a callback that waits, for example for the user of the
    /// connected client, can check this to stop waiting early.
    pub fn is_pending(&self) -> bool {
        self.0.is_pending()
    }
}

impl std::fmt::Debug for ConnectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectRequest")
            .field("address", &self.address())
            .field("pending", &self.is_pending())
            .finish()
    }
}
