mod client_base;
// mod data;
mod graphs;
#[cfg(not(feature = "build-script"))]
mod handle_message;
mod image;
mod list;
mod map;
mod sender;
mod values;
pub mod values_info;

#[cfg(not(feature = "build-script"))]
mod states_creator;

#[cfg(feature = "build-script")]
pub mod build_script;

#[cfg(all(any(feature = "client", feature = "client-wasm"), not(feature = "build-script")))]
mod client;

#[cfg(all(any(feature = "client", feature = "client-wasm"), not(feature = "build-script")))]
pub use client::ClientBuilder;

#[cfg(feature = "client")]
mod websocket;

#[cfg(feature = "client-wasm")]
mod websocket_wasm;

pub use client_base::{Client, ConnectionState};
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use map::ValueMap;
pub use values::{Diff, Signal, Value, ValueStatic};

#[cfg(not(feature = "build-script"))]
pub use states_creator::StatesCreator;

#[cfg(feature = "build-script")]
pub use build_script::state_creator::StatesCreator;

pub trait State {
    fn new(c: &mut StatesCreator, parent: String) -> Self;
}

pub use egui_states_macros::{state_enum, state_struct};
