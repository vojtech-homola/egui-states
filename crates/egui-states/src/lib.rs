extern crate self as egui_states;

mod collections;
mod event_async;
mod graphs;
mod hashing;
mod image;
mod serialization;
mod transport;

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
    atomics::{Atomic, AtomicLock, AtomicLockStatic, AtomicStatic, FallbackLock, UpdateLock},
    client::ClientBuilder,
    client::{Client, ConnectionState},
    graphs::ValueGraphs,
    image::ValueImage,
    list::ValueVec,
    map::ValueMap,
    states_creator::StatesCreator,
    values::{
        Diff, DiffAtomic, GetQueueType, NoQueue, Queue, Signal, Static, StaticAtomic, Value,
        ValueAtomic,
    },
};

#[cfg(feature = "client")]
pub trait State {
    const NAME: &'static str;

    fn new(c: &mut impl StatesCreator) -> Self;
}

#[cfg(feature = "client")]
pub use egui_states_macros::State;
pub use egui_states_macros::Transportable;
pub use serde;
pub use transport::{InitValue, ObjectType, Transportable};

pub(crate) const PROTOCOL_VERSION: u16 = 4;
