//! The WebSocket connection to the server, native or in the browser.

pub(crate) mod timer;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod websocket;
#[cfg(target_arch = "wasm32")]
pub(crate) mod websocket_wasm;
