//! Native Rust server API, available with the `server` feature.
//!
//! Applications normally use bindings produced by
//! `generate_rust`. For manual construction, create a
//! [`StateServer`](crate::server::StateServer), register every state handle,
//! call [`StateServer::finalize`](crate::server::StateServer::finalize), and
//! then [`StateServer::start`](crate::server::StateServer::start). Retain every
//! [`CallbackHandle`](crate::server::CallbackHandle) for as long as its
//! callback should remain registered.
//!
//! Only `StateServer` clones keep the server alive: dropping the last one stops
//! it, even while state handles or callback handles remain. Callbacks may
//! therefore capture state handles freely. A callback that captures a
//! `StateServer` clone keeps the server running until its `CallbackHandle` is
//! dropped.

mod callbacks;
mod collections;
mod data;
mod error;
mod file_transfer;
mod image;
mod logging;
mod options;
mod state_server;
mod values;

pub use crate::file_transfer::{
    TransferError, TransferHandle, TransferMeta, TransferProgress, TransferStatus,
};
pub use callbacks::CallbackHandle;
pub use collections::{MapState, VecState};
pub use data::{Data, DataElement, DataMulti, DataMultiTake, DataTake, HeaderValue};
pub use error::{Result, ServerError};
pub use file_transfer::{Download, OfferedFile, TransferOffer, Upload};
pub use image::{Image, ImageColor, ImageFormat, ImageMulti};
pub use logging::{LogLevel, LoggingSignal};
pub use options::{ErrorHandler, ServerOptions};
pub use state_server::StateServer;
pub use values::{Signal, Static, Value, ValueTake};
