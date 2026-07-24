extern crate self as egui_states;

mod collections;
mod data_transport;
mod event;
mod hashing;
mod image_transport;
mod serialization;
mod typed;

#[cfg(feature = "build_scripts")]
pub mod build_scripts;
#[cfg(feature = "client")]
mod client;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "server")]
pub mod server;
#[cfg(any(feature = "server", feature = "python"))]
mod server_core;

#[cfg(feature = "client")]
pub use client::{
    atomics::{Atomic, AtomicLock, AtomicLockStatic, AtomicStatic, FallbackLock, UpdateLock},
    client::ClientBuilder,
    client::{Client, ConnectionState},
    data::{Data, DataMulti},
    data_take::{DataMultiTake, DataTake},
    image::Image,
    initial_value::{InitValue, InitialValue},
    states_creator::StatesCreator,
    value_map::MapState,
    value_vec::VecState,
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

pub use egui_states_macros::Typed;
#[cfg(feature = "client")]
pub use egui_states_macros::{Atomic, AtomicStatic, InitialValue, State};
pub use serde;
pub use typed::{ObjectType, Typed};

pub(crate) const PROTOCOL_VERSION: u16 = 5;
