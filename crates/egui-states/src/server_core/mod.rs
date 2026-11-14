pub(crate) mod error;
pub(crate) mod file_transfer;
pub(crate) mod http;
pub(crate) mod net;
#[cfg(test)]
mod packer_reference;
pub(crate) mod server;
pub(crate) mod signals;
pub(crate) mod states;
pub(crate) mod transfer;

#[cfg(all(test, feature = "client"))]
pub(crate) use net::session::MAX_PENDING_CONNECTIONS;
