//! Build-time generation of server bindings from a client [`crate::State`].
//!
//! Call a generator from the UI or shared-state crate's `build.rs`. Both
//! generators create their output directory, avoid rewriting unchanged files,
//! and emit a generated-file warning in every output file.

// Experimental: the C header generator is part of the `c_abi` feature.
#[cfg(feature = "c_abi")]
mod c;
mod python;
mod rust;
mod scripts;
mod states_creator_build;

#[cfg(feature = "c_abi")]
pub use c::generate_c;
pub use python::generate_python;
pub use rust::generate_rust;
