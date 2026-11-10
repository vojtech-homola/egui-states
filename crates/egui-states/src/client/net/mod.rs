//! The WebSocket connection to the server, native or in the browser.

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod websocket;
#[cfg(target_arch = "wasm32")]
pub(crate) mod websocket_wasm;
