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

/// The window of the default profile, `TransferProfile::Balanced`.
pub(crate) const FILE_TRANSFER_WINDOW: u64 = 256 * 1024;

/// The chunk size of the default profile, `TransferProfile::Balanced`.
pub(crate) const FILE_TRANSFER_CHUNK: usize = 32 * 1024;

/// How file data flows on one connection. The server chooses them and
/// announces them to the client, so both peers use the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TransferLimits {
    /// Bytes of file data the sender may have in flight before the receiver
    /// returns credit. A state message waits behind at most about
    /// `window / link rate` of file data, and a transfer moves at most one
    /// window per round trip.
    pub(crate) window: u64,
    /// Payload bytes of one data chunk. The receiver returns credit chunk by
    /// chunk, so a smaller chunk smooths the flow on slow links and costs
    /// throughput on fast ones.
    pub(crate) chunk: usize,
}

impl TransferLimits {
    /// The largest chunk a peer accepts; a chunk travels in one frame.
    pub(crate) const MAX_CHUNK: usize = 1024 * 1024;

    /// Checks limits that came from the other peer.
    #[cfg_attr(not(feature = "client"), allow(dead_code))]
    pub(crate) fn new(window: u64, chunk: usize) -> Result<Self, String> {
        if chunk == 0 || chunk > Self::MAX_CHUNK || chunk as u64 > window {
            return Err(format!(
                "invalid file transfer limits: window {window} bytes, chunk {chunk} bytes"
            ));
        }
        Ok(Self { window, chunk })
    }
}

impl Default for TransferLimits {
    fn default() -> Self {
        Self {
            window: FILE_TRANSFER_WINDOW,
            chunk: FILE_TRANSFER_CHUNK,
        }
    }
}

/// How file transfers share the connection with state synchronization.
///
/// File data takes a low-priority lane: a state change is never queued behind
/// more than one window of it. A larger window lets transfers fill faster and
/// more distant links, but a state change may then wait longer behind file
/// data. The server's profile applies to the connection in both directions.
#[cfg(any(feature = "server", feature = "python"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferProfile {
    /// For slow links (VPN, mobile, under about 20 Mbit/s), where the UI must
    /// stay responsive during transfers. At most 64 KiB of file data waits
    /// ahead of a state change, but a transfer moves at most 64 KiB per round
    /// trip, about 3 MB/s at 20 ms.
    Responsive,
    /// The default: close to full speed on links from 100 Mbit/s up, with at
    /// most 256 KiB of file data ahead of a state change. On slow links the
    /// wait becomes noticeable, about 0.2 s at 10 Mbit/s.
    #[default]
    Balanced,
    /// The fastest transfers, for a client on the same machine or when
    /// transfers matter more than the UI. On a network, a state change may
    /// wait behind up to 1 MiB of file data, about 8 ms at 1 Gbit/s.
    Throughput,
}

#[cfg(any(feature = "server", feature = "python"))]
impl TransferProfile {
    pub(crate) fn limits(self) -> TransferLimits {
        let (window, chunk) = match self {
            Self::Responsive => (64 * 1024, 16 * 1024),
            Self::Balanced => (FILE_TRANSFER_WINDOW, FILE_TRANSFER_CHUNK),
            Self::Throughput => (1024 * 1024, 128 * 1024),
        };
        TransferLimits { window, chunk }
    }

    /// The profile of a number, the value of the Python `TransferProfile`.
    #[cfg(feature = "python")]
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Responsive),
            1 => Some(Self::Balanced),
            2 => Some(Self::Throughput),
            _ => None,
        }
    }
}

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

#[cfg(all(test, any(feature = "server", feature = "python")))]
mod tests {
    use super::*;

    /// A client accepts the limits of every profile, and they grow from
    /// responsive to throughput.
    #[test]
    fn every_profile_has_limits_a_client_accepts() {
        let profiles = [
            TransferProfile::Responsive,
            TransferProfile::Balanced,
            TransferProfile::Throughput,
        ];
        for profile in profiles {
            let limits = profile.limits();
            assert_eq!(TransferLimits::new(limits.window, limits.chunk), Ok(limits));
        }
        for pair in profiles.windows(2) {
            let (smaller, larger) = (pair[0].limits(), pair[1].limits());
            assert!(smaller.window < larger.window && smaller.chunk < larger.chunk);
        }
        assert_eq!(
            TransferProfile::default().limits(),
            TransferLimits::default()
        );
    }

    /// The numbers are the values of the Python `TransferProfile` enum.
    #[cfg(feature = "python")]
    #[test]
    fn python_profile_numbers_match_the_python_enum() {
        let profiles = [
            TransferProfile::Responsive,
            TransferProfile::Balanced,
            TransferProfile::Throughput,
        ];
        for (code, profile) in profiles.into_iter().enumerate() {
            assert_eq!(TransferProfile::from_code(code as u8), Some(profile));
        }
        assert_eq!(TransferProfile::from_code(3), None);
    }
}
