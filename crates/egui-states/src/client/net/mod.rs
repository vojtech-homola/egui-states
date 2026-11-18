//! The WebSocket connection to the server, native or in the browser.

#[cfg(all(feature = "tls", not(target_arch = "wasm32")))]
pub(crate) mod tls;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod websocket;
#[cfg(target_arch = "wasm32")]
pub(crate) mod websocket_wasm;

/// Where and how the client connects.
pub(crate) struct Target {
    /// The WebSocket URL.
    pub(crate) url: String,
    /// The host the TCP connection goes to: a name or an IP address, without
    /// brackets.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) host: String,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) port: u16,
    /// `None` connects without TLS.
    #[cfg(all(feature = "tls", not(target_arch = "wasm32")))]
    pub(crate) tls: Option<crate::client::tls::Tls>,
}
