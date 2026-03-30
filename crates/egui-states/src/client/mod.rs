mod event;
mod handle_message;
pub(crate) mod sender;

pub mod atomics;
pub mod client;
pub mod graphs;
pub mod image;
pub mod list;
pub mod map;
pub mod states_creator;
pub mod values;

#[cfg(not(target_arch = "wasm32"))]
mod websocket;
#[cfg(target_arch = "wasm32")]
mod websocket_wasm;

pub(crate) mod private {
    pub trait IsBlocking: Send + Sync {
        const IS_BLOCKING: bool;
    }
}

pub struct Blocking;
impl private::IsBlocking for Blocking {
    const IS_BLOCKING: bool = true;
}

pub struct NonBlocking;
impl private::IsBlocking for NonBlocking {
    const IS_BLOCKING: bool = false;
}
