mod handle_message;
pub(crate) mod sender;

pub mod client;
pub mod graphs;
pub mod image;
pub mod list;
pub mod map;
pub mod states_creator;
pub mod values;
pub mod values_atomic;

#[cfg(not(target_arch = "wasm32"))]
mod websocket;
#[cfg(target_arch = "wasm32")]
mod websocket_wasm;
