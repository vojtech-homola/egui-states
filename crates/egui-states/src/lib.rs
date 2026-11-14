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
//! // In an eframe app, pass `CreationContext::egui_ctx` instead.
//! let context = egui::Context::default();
//! let (state, client) = ClientBuilder::<AppState>::new(context, 8091).build();
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
//! use the same state API. The experimental `c_abi` feature supports the
//! `egui_states_c` library and, with `build_scripts`, `generate_c`.
//!
//! See the [project README](https://github.com/vojtech-homola/egui-states) for
//! complete setup instructions and runnable examples.

extern crate self as egui_states;

#[cfg(test)]
mod test_support;

mod event;
#[cfg(any(feature = "client", feature = "server", feature = "python"))]
// Not used by the state endpoints yet.
#[allow(dead_code, unused_imports)]
mod file_transfer;
mod hashing;
mod protocol;
mod typed;

/// Internal entry points for the benchmarks; not a public API.
#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod bench_support;
/// Build-time generators for typed Rust and Python server bindings.
#[cfg(feature = "build_scripts")]
pub mod build_scripts;
/// Experimental support for the `egui_states_c` crate; not a stable Rust API.
#[cfg(feature = "c_abi")]
#[doc(hidden)]
pub mod c_abi;
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
    client::ClientBuilder,
    client::{Client, ConnectionState},
    states::{
        atomics::{Atomic, AtomicLock, AtomicLockStatic, AtomicStatic, FallbackLock, UpdateLock},
        data::{Data, DataMulti, HeaderValue},
        data_take::{DataMultiTake, DataTake},
        file_transfer::{Download, Upload},
        image::{Image, ImageMulti},
        initial_value::{InitValue, InitialValue},
        map::MapState,
        values::{
            Diff, DiffAtomic, GetQueueType, NoQueue, Queue, Signal, Static, StaticAtomic, Value,
            ValueAtomic, ValueTake,
        },
        vec::VecState,
    },
    states_creator::StatesCreator,
};
#[cfg(feature = "client")]
pub use file_transfer::{
    TransferError, TransferHandle, TransferMeta, TransferProgress, TransferStatus,
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
pub(crate) const PROTOCOL_VERSION: u16 = 7;
