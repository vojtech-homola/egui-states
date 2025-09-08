mod client_state;
mod dict;
mod event;
mod graphs;
mod handle_message;
mod image;
mod list;
mod sender;
mod states_creator;
mod values;

#[cfg(feature = "client")]
// #[cfg(not(feature = "client-wasm"))]
mod client;

#[cfg(feature = "client")]
// #[cfg(not(feature = "client-wasm"))]
pub use client::ClientBuilder;

#[cfg(feature = "client-wasm")]
mod client_wasm;

#[cfg(feature = "client-wasm")]
pub use client_wasm::ClientBuilder;

pub use client_state::{ConnectionState, UIState};
pub use dict::ValueDict;
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use states_creator::ValuesCreator;
pub use values::{Diff, Signal, Value, ValueStatic};

pub trait UpdateValue: Sync + Send {
    fn update_value(&self, data: &[u8]) -> Result<bool, String>;
}

pub trait State {
    fn new(c: &mut ValuesCreator) -> Self;
}
