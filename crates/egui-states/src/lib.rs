mod client_base;
// mod data;
mod graphs;
mod handle_message;
mod image;
mod list;
mod map;
mod sender;
mod values;
mod values_creator;

pub mod build_scripts;

#[cfg(any(feature = "client", feature = "client-wasm"))]
mod client;

#[cfg(any(feature = "client", feature = "client-wasm"))]
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
pub use parser_values::ParseValuesCreator;
pub use values::{Diff, Signal, Value, ValueStatic};
pub use values_creator::{ClientValuesCreator, ValuesCreator};

pub trait State {
    const N: &'static str;

    fn new(c: &mut impl ValuesCreator) -> Self;
}

mod parser;
mod parser_values;

pub use egui_states_macros::{state_enum, state_struct};
pub use parser::{GetInitValue, GetTypeInfo, InitValue, TypeInfo};
