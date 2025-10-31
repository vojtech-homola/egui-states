mod client_base;
mod data;
mod dict;
mod graphs;
mod handle_message;
mod image;
mod list;
mod sender;
mod values;
mod values_creator;

pub mod build_scripts;

#[cfg(feature = "client")]
mod client;

#[cfg(feature = "client")]
pub use client::ClientBuilder;

#[cfg(feature = "client-wasm")]
mod client_wasm;

#[cfg(feature = "client-wasm")]
pub use client_wasm::ClientBuilder;

pub use client_base::{Client, ConnectionState};
pub use dict::ValueDict;
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use parser_values::ParseValuesCreator;
pub use values::{Diff, Signal, Value, ValueStatic};
pub use values_creator::{ClientValuesCreator, ValuesCreator};

pub trait UpdateValue: Sync + Send {
    fn update_value(&self, data: &[u8]) -> Result<bool, String>;
}

pub trait State {
    const N: &'static str;

    fn new(c: &mut impl ValuesCreator) -> Self;
}

mod parser;
mod parser_values;

pub use egui_states_macros::{state_enum, state_struct};
pub use parser::{GetInitValue, GetTypeInfo, InitValue, TypeInfo};
