//! C ABI for the egui-states server.
//!
//! The exported functions and `include/egui_states.h` form a low-level
//! gateway for server wrappers in other languages. Every rule of the contract
//! lives in `egui_states::c_abi`; this crate only checks and converts
//! arguments, catches panics, and reports errors as status codes.
//!
//! Item names equal their C names, so cbindgen generates the header without
//! renaming. Safety requirements are documented once, in the header.
#![allow(
    non_camel_case_types,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

mod codec;
mod events;
mod ffi;
mod media;
mod meta;
mod server;
mod states;

pub use codec::*;
pub use events::*;
pub use ffi::{egui_states_status_t, egui_states_status_t::*};
pub use media::*;
pub use meta::*;
pub use server::*;
pub use states::*;
