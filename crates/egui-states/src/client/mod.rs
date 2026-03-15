mod handle_message;
pub(crate) mod sender;

pub(crate) mod atomics;
pub(crate) mod client;
pub(crate) mod graphs;
pub(crate) mod image;
pub(crate) mod list;
pub(crate) mod map;
pub(crate) mod states_creator;
pub(crate) mod values;
pub(crate) mod data;

#[cfg(not(target_arch = "wasm32"))]
mod websocket;
#[cfg(target_arch = "wasm32")]
mod websocket_wasm;
