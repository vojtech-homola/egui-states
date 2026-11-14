//! Build-time generation of server bindings from a client [`crate::State`].
//!
//! Call a generator from the UI or shared-state crate's `build.rs`. Both
//! generators create their output directory, avoid rewriting unchanged files,
//! and emit a generated-file warning in every output file.

mod python;
mod rust;
mod scripts;
mod states_creator_build;

pub use python::generate_python;
pub use rust::generate_rust;
