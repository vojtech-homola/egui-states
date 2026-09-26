//! Typed state synchronization between an [`egui`](https://docs.rs/egui) client
//! and a Rust or Python server.
//!
//! # Workflow
//!
//! 1. Describe the UI state as a Rust struct and derive [`State`](derive@State).
//! 2. Construct the client state tree with [`ClientBuilder`].
//! 3. Generate matching Python or Rust server bindings from the same type in a
//!    build script.
//! 4. Start the server and call [`Client::connect`]. The peers compare their
//!    wire-protocol version and any configured application version or token
//!    before synchronizing values over a WebSocket connection. A generated
//!    layout hash is available for applications that want to use it as the
//!    application version.
//!
//! ```no_run
//! use egui_states::{ClientBuilder, Value};
//!
//! #[derive(egui_states::State)]
//! struct AppState {
//!     counter: Value<i32>,
//! }
//!
//! let (state, client) = ClientBuilder::<AppState>::new().build(8091);
//! client.connect();
//! state.counter.set_signal(state.counter.get() + 1);
//! ```
//!
//! # State kinds
//!
//! - [`Value`] and [`ValueAtomic`] are bidirectional values.
//! - [`Static`] and [`StaticAtomic`] are controlled by the server and read by
//!   the client.
//! - [`Signal`] is a client-to-server event without retained client state.
//! - [`ValueTake`], [`DataTake`], and [`DataMultiTake`] are one-shot
//!   server-to-client transfers.
//! - [`VecState`] and [`MapState`] are server-controlled collections.
//! - [`Data`] and [`DataMulti`] efficiently transfer numeric buffers.
//! - [`Image`] updates one egui texture, while [`ImageMulti`] manages a sparse
//!   collection of textures indexed by `u32` keys.
//!
//! # Cargo features
//!
//! `client` is enabled by default. Enable `server` for the native Rust server
//! API, `python` when building the PyO3 extension module, or `build_scripts` for
//! `generate_python` and `generate_rust`. Native and WebAssembly egui clients
//! use the same state API.
//!
//! See the [project README](https://github.com/vojtech-homola/egui-states) for
//! complete setup instructions and runnable examples.

extern crate self as egui_states;

#[cfg(test)]
mod test_support;

mod collections;
mod data_transport;
mod event;
mod hashing;
mod image_transport;
mod serialization;
mod typed;

/// Build-time generators for typed Rust and Python server bindings.
#[cfg(feature = "build_scripts")]
pub mod build_scripts;
#[cfg(feature = "client")]
mod client;
/// Python extension-module support used by the `egui-states` Python package.
#[cfg(feature = "python")]
pub mod python;
/// Native Rust server API.
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
    image::{Image, ImageMulti},
    initial_value::{InitValue, InitialValue},
    states_creator::StatesCreator,
    value_map::MapState,
    value_vec::VecState,
    values::{
        Diff, DiffAtomic, GetQueueType, NoQueue, Queue, Signal, Static, StaticAtomic, Value,
        ValueAtomic, ValueTake,
    },
};

/// A root or nested group of client-side states.
///
/// Prefer deriving this trait with [`State`](derive@State). Field names become
/// segments in the synchronized state path, starting below `root`.
#[cfg(feature = "client")]
pub trait State {
    /// Stable Rust type name used for the corresponding generated server type.
    ///
    /// The derive uses the Rust struct name. Regenerate server bindings after
    /// changing it so both source APIs use the same generated type names.
    const NAME: &'static str;

    /// Constructs the state and registers all of its children with `creator`.
    fn new(c: &mut impl StatesCreator) -> Self;
}

pub use egui_states_macros::typed;
#[cfg(feature = "client")]
pub use egui_states_macros::{Atomic, AtomicStatic, InitialValue, State};
/// Serde re-export used by [`typed`] and generated implementations.
///
/// Generated derives refer to this module, so applications using [`typed`] do
/// not need to add a separate direct Serde dependency.
pub use serde;
pub use typed::{ObjectType, RustDerive, Typed};

// Wire protocol version advertised during the WebSocket handshake. A mismatch
// rejects the connection; bump this only for an incompatible message change.
pub(crate) const PROTOCOL_VERSION: u16 = 6;
