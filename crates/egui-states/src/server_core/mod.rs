pub(crate) mod core;
mod sender;
mod socket_reader;

pub(crate) mod data_core;
pub(crate) mod data_take_core;
pub(crate) mod error;
pub(crate) mod heartbeat;
pub(crate) mod image_core;
pub(crate) mod image_core_common;
pub(crate) mod image_multi_core;
pub(crate) mod map_core;
pub(crate) mod origin;
#[cfg(test)]
mod packer_reference;
pub(crate) mod server;
pub(crate) mod signals;
pub(crate) mod transfer;
pub(crate) mod transport;
pub(crate) mod values_core;
pub(crate) mod vec_core;
pub(crate) mod web;

#[cfg(all(test, feature = "client"))]
pub(crate) use core::MAX_PENDING_CONNECTIONS;
