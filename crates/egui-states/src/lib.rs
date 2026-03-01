// mod data;
mod graphs;
mod build_scripts;
mod handle_message;
mod image;
mod initial_value;
mod list;
mod map;
mod sender;
mod states_creator;
mod values;
mod values_atomic;

mod client;

pub use client::ClientBuilder;

#[cfg(not(target_arch = "wasm32"))]
mod websocket;

#[cfg(target_arch = "wasm32")]
mod websocket_wasm;

pub use client::{Client, ConnectionState};
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use initial_value::{GetInitValue, InitValue};
pub use list::ValueList;
pub use map::ValueMap;
pub use states_creator::StatesCreator;
pub use values::{
    Diff, DiffAtomic, GetQueueType, NoQueue, Queue, Signal, Static, StaticAtomic, Value,
    ValueAtomic,
};

pub use values_atomic::{Atomic, AtomicLock};

pub trait State {
    const NAME: &'static str;

    fn new(c: &mut impl StatesCreator) -> Self;
}

pub use egui_states_core::types::{GetType, ObjectType};
pub use egui_states_macros::{state_enum, state_struct};
pub use serde;
