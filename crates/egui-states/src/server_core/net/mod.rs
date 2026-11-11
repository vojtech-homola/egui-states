//! Connections: accepting them, their handshake, and the session that
//! carries the messages of the connected client.

pub(crate) mod heartbeat;
pub(super) mod sender;
pub(crate) mod session;
mod socket_reader;
pub(crate) mod transport;
