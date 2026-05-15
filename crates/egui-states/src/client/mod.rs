mod event;

pub(crate) mod atomics;
pub(crate) mod client;
pub(crate) mod data;
pub(crate) mod data_multi;
pub(crate) mod image;
pub(crate) mod messages;
pub(crate) mod states_creator;
pub(crate) mod value_map;
pub(crate) mod value_vec;
pub(crate) mod values;

#[cfg(not(target_arch = "wasm32"))]
mod websocket;
#[cfg(target_arch = "wasm32")]
mod websocket_wasm;
