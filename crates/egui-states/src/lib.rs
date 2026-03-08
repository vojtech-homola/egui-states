mod collections;
mod event_async;
mod graphs;
mod image;
mod initial_value;
mod hashing;
mod serialization;
mod types;

#[cfg(feature = "build_scripts")]
pub mod build_scripts;
#[cfg(feature = "client")]
mod client;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "client")]
pub use client::{
    client::ClientBuilder,
    client::{Client, ConnectionState},
    graphs::ValueGraphs,
    image::ValueImage,
    list::ValueList,
    map::ValueMap,
    states_creator::StatesCreator,
    values::{
        Diff, DiffAtomic, GetQueueType, NoQueue, Queue, Signal, Static, StaticAtomic, Value,
        ValueAtomic,
    },
    values_atomic::{Atomic, AtomicLock, AtomicLockStatic, AtomicStatic, FallbackLock, UpdateLock},
};

#[cfg(feature = "client")]
pub trait State {
    const NAME: &'static str;

    fn new(c: &mut impl StatesCreator) -> Self;
}

pub use crate::types::{GetType, ObjectType};
pub use egui_states_macros::{StateEnum, StateStruct};
pub use initial_value::{GetInitValue, InitValue};
pub use serde;

pub(crate) const PROTOCOL_VERSION: u16 = 2;
