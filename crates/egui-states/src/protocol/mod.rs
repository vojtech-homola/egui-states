//! The wire format both peers share.
//!
//! Each peer's side of it is a module of its own, gated by the features that
//! build that peer.

mod fastvec;
mod header;
mod limits;

#[cfg(feature = "client")]
mod client_side;
#[cfg(any(feature = "server", feature = "python"))]
mod server_side;

pub(crate) mod collections;
pub(crate) mod data;
pub(crate) mod image;

#[cfg(feature = "client")]
pub(crate) use client_side::*;
pub(crate) use fastvec::*;
pub(crate) use header::*;
pub(crate) use limits::*;
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) use server_side::*;
