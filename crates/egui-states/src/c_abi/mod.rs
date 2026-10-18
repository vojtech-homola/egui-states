//! Safe implementation of the C ABI exported by the `egui_states_c` crate.
//!
//! The C ABI is a low-level gateway for server wrappers in other languages. It
//! registers states at runtime from Rust-syntax type strings, moves values
//! through type-checked [`Writer`](crate::c_abi::Writer) and
//! [`Reader`](crate::c_abi::Reader) handles, and delivers client changes by
//! polling [`CServer::wait_event`](crate::c_abi::CServer::wait_event). This
//! module holds every rule of that contract so the exported functions only
//! marshal arguments.
//!
//! This is not a stable Rust API; the stable boundary is the C header.

mod error;
mod events;
mod kinds;
mod media;
mod reader;
mod server;
mod tree;
mod type_parser;
mod types;
mod writer;

pub use error::{Error, ErrorCode, Result};
pub use events::ChangeEvent;
pub use kinds::{DataKind, StateKind, ValueKind};
pub use media::{ImageFormat, ImageInput};
pub use reader::Reader;
pub use server::CServer;
pub use writer::Writer;

use crate::typed::ObjectType;

/// Version of the C ABI contract.
///
/// It changes whenever a released function changes signature or behavior.
pub const ABI_VERSION: u32 = 1;

/// Returns the wire-protocol version advertised during the handshake.
pub fn protocol_version() -> u16 {
    crate::PROTOCOL_VERSION
}

/// Returns the protocol id of the state with the full dotted `name`.
///
/// Clients and every server compute the same id from the same name.
pub fn state_id(name: &str) -> u64 {
    crate::hashing::generate_value_id(name)
}

/// Formats `object_type` in the type syntax accepted by [`CServer`].
pub fn type_expression(object_type: &ObjectType) -> String {
    crate::typed::type_expression(object_type)
}

/// Reserved id of client log messages, typed `(u8, String)`.
pub const LOGGING_ID: u64 = crate::server_core::signals::LOGGING_ID;
/// Reserved id of client connections, carrying the peer address as `String`.
pub const ON_CONNECT_ID: u64 = crate::server_core::signals::ON_CONNECT_ID;
/// Reserved id of client disconnections, typed `()`.
pub const ON_DISCONNECT_ID: u64 = crate::server_core::signals::ON_DISCONNECT_ID;
/// Reserved id of diagnostic client messages, typed `String`.
pub const CLIENT_MESSAGE_ID: u64 = crate::server_core::signals::CLIENT_MESSAGE_ID;
