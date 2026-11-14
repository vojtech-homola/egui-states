//! File transfer endpoints: the engine both peers share.
//!
//! A `Download` endpoint sends files from the server to the client and an
//! `Upload` endpoint from the client to the server. Each peer runs one
//! [`engine::Engine`] that is the sender of its outgoing transfers and the
//! receiver of the incoming ones; only how a connection is attached and how
//! frames are encoded differ between the peers.
//!
//! File data travels in a low-priority lane of the existing connection: the
//! writer of each peer sends a chunk only when no state message is waiting. A
//! credit window bounds the data between the two applications, so a state
//! message waits behind at most one window of file data.

pub(crate) mod engine;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod fs;
pub(crate) mod names;
pub(crate) mod shared;
pub(crate) mod sink;
pub(crate) mod source;
#[cfg(target_arch = "wasm32")]
pub(crate) mod web;

use std::fmt;

#[cfg(any(feature = "client", feature = "server"))]
use serde::{Serialize, de::DeserializeOwned};

#[cfg(any(feature = "client", feature = "server"))]
use crate::typed::Typed;

#[cfg(any(feature = "client", feature = "server"))]
pub use shared::TransferHandle;

/// Bytes of file data the sender may have in flight before the receiver
/// returns credit. It bounds how long a state message waits behind file data:
/// about `FILE_TRANSFER_WINDOW / link rate`.
pub(crate) const FILE_TRANSFER_WINDOW: u64 = 256 * 1024;

/// Payload bytes of one data chunk.
pub(crate) const FILE_TRANSFER_CHUNK: usize = 64 * 1024;

/// The metadata type of a `Download<M>` or `Upload<M>` endpoint.
///
/// Any type made with `#[egui_states::typed]` qualifies, and `()` for an
/// endpoint without metadata. An encoded value may be at most 1 MB, the size
/// limit of a value.
#[cfg(any(feature = "client", feature = "server"))]
pub trait TransferMeta: Typed + Serialize + DeserializeOwned + Send + Sync + 'static {}

#[cfg(any(feature = "client", feature = "server"))]
impl<M> TransferMeta for M where M: Typed + Serialize + DeserializeOwned + Send + Sync + 'static {}

/// The wire type id of the metadata type `M`; 0 for an endpoint without
/// metadata.
#[cfg(any(feature = "client", feature = "server"))]
pub(crate) fn meta_type_id<M: Typed>() -> u32 {
    crate::protocol::data::header_type_id(&M::get_type()).unwrap_or(0)
}

/// Where a transfer stands.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferStatus {
    /// Waiting for a file to be picked, for the other side's answer, or
    /// behind a transfer of another endpoint.
    Pending,
    /// Data is flowing.
    Running,
    /// The receiver has saved everything.
    Done,
    /// The transfer ended without success.
    Failed(TransferError),
}

/// How far a transfer has come.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferProgress {
    /// Bytes across all files.
    pub done: u64,
    /// Bytes in total; always known in this version.
    pub total: Option<u64>,
    /// Files completely transferred.
    pub files_done: u32,
    /// Files in the transfer.
    pub files_total: u32,
}

impl TransferProgress {
    /// The completed fraction in `0.0..=1.0`, or `None` while the total is
    /// unknown. A transfer of zero bytes counts as complete.
    #[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total?;
        if total == 0 {
            return Some(1.0);
        }
        Some((self.done as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }
}

/// Why a transfer failed or a send was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferError {
    /// The endpoint already has an unfinished transfer.
    Busy,
    /// No client is connected, or the client is not connected to a server.
    NotConnected,
    /// The connection ended before the transfer finished.
    Disconnected,
    /// The receiver rejected the offer, with its reason.
    Rejected(String),
    /// A side cancelled the transfer.
    Cancelled {
        /// Whether the other side cancelled.
        by_peer: bool,
        /// The reason, if one was given.
        reason: Option<String>,
    },
    /// The data exceeds the limit of an in-memory accept.
    TooLarge,
    /// The offer cannot be accepted that way, or a file name cannot be used
    /// on this system.
    Unsupported(String),
    /// The metadata could not be encoded or decoded, or is too large.
    Meta(String),
    /// Reading or writing a file failed.
    Io(String),
    /// The other side broke the transfer protocol.
    Protocol(String),
    /// A wait on the server ran out.
    Timeout,
    /// The input of a server send is invalid.
    InvalidInput(String),
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("the endpoint already has an unfinished transfer"),
            Self::NotConnected => f.write_str("not connected"),
            Self::Disconnected => f.write_str("the connection ended"),
            Self::Rejected(reason) if reason.is_empty() => f.write_str("rejected"),
            Self::Rejected(reason) => write!(f, "rejected: {reason}"),
            Self::Cancelled { by_peer, reason } => {
                f.write_str(if *by_peer {
                    "cancelled by the other side"
                } else {
                    "cancelled"
                })?;
                match reason {
                    Some(reason) if !reason.is_empty() => write!(f, ": {reason}"),
                    _ => Ok(()),
                }
            }
            Self::TooLarge => f.write_str("the data exceeds the size limit"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::Meta(message) => write!(f, "invalid metadata: {message}"),
            Self::Io(message) => write!(f, "I/O error: {message}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
            Self::Timeout => f.write_str("timed out"),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
        }
    }
}

impl std::error::Error for TransferError {}

impl TransferError {
    /// The code and message a failure travels with in a `Finished` frame, or
    /// `None` for kinds that travel otherwise.
    pub(crate) fn wire(&self) -> (crate::protocol::file_transfer::ErrorCode, String) {
        use crate::protocol::file_transfer::ErrorCode;
        match self {
            Self::Io(message) => (ErrorCode::Io, message.clone()),
            Self::TooLarge => (ErrorCode::TooLarge, String::new()),
            Self::Unsupported(message) => (ErrorCode::Unsupported, message.clone()),
            Self::Meta(message) => (ErrorCode::Meta, message.clone()),
            other => (ErrorCode::Protocol, other.to_string()),
        }
    }

    /// The failure a `Finished` frame from the other side reports.
    pub(crate) fn from_wire(code: u8, message: String) -> Self {
        use crate::protocol::file_transfer::ErrorCode;
        match ErrorCode::from_u8(code) {
            ErrorCode::Io => Self::Io(message),
            ErrorCode::TooLarge => Self::TooLarge,
            ErrorCode::Unsupported => Self::Unsupported(message),
            ErrorCode::Meta => Self::Meta(message),
            ErrorCode::Protocol => Self::Protocol(message),
        }
    }
}
