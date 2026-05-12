extern crate self as egui_states;

mod collections;
mod data_transport;
mod event_async;
mod hashing;
mod image_header;
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
    data::{Data, DataMulti, DataTake},
    image::ValueImage,
    states_creator::StatesCreator,
    value_map::ValueMap,
    value_vec::ValueVec,
    values::{
        Diff, DiffAtomic, GetQueueType, NoQueue, Queue, Signal, Static, StaticAtomic, Value,
        ValueAtomic, ValueTake,
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
