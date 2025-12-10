mod client_base;
// mod data;
pub mod build_script;
mod client_states;
mod graphs;
mod handle_message;
mod image;
mod list;
mod map;
mod sender;
mod states_creator;
mod values;

#[cfg(any(feature = "client", feature = "client-wasm"))]
mod client;

#[cfg(any(feature = "client", feature = "client-wasm"))]
pub use client::ClientBuilder;

#[cfg(feature = "client")]
mod websocket;

#[cfg(feature = "client-wasm")]
mod websocket_wasm;

pub use build_script::values_info::{GetInitValue, InitValue};
pub use client_base::{Client, ConnectionState};
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use map::ValueMap;
pub use states_creator::{StatesBuilder, StatesCreator};
pub use values::{Diff, Signal, Value, ValueStatic};

pub trait State {
    fn new(c: &mut impl StatesCreator, parent: String) -> Self;
}

pub use egui_states_core::types::{GetType, ObjectType};
pub use egui_states_macros::{state_enum, state_struct};
pub use serde;
