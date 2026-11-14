//! Where the receiver of a transfer puts its data.

use crate::file_transfer::TransferError;

/// The destination of an incoming transfer.
pub(crate) enum Sink {
    /// Collects the data in memory, up to `limit` bytes.
    Memory { data: Vec<u8>, limit: u64 },
    /// Drops the data, for benchmarks.
    #[cfg(any(test, feature = "bench"))]
    Discard,
}

impl Sink {
    pub(crate) fn memory(limit: u64) -> Self {
        Self::Memory {
            data: Vec::new(),
            limit,
        }
    }

    /// Checks before the accept that the sink can take an offer of `files`
    /// files and `total` bytes.
    pub(crate) fn check(&self, files: usize, total: u64) -> Result<(), TransferError> {
        match self {
            Self::Memory { limit, .. } => {
                if files != 1 {
                    return Err(TransferError::Unsupported(format!(
                        "the offer has {files} files; accept it into a folder"
                    )));
                }
                if total > *limit {
                    return Err(TransferError::TooLarge);
                }
                Ok(())
            }
            #[cfg(any(test, feature = "bench"))]
            Self::Discard => Ok(()),
        }
    }

    /// Whether the sink takes data without blocking, so the engine writes to
    /// it while it handles the frame.
    pub(crate) fn is_inline(&self) -> bool {
        match self {
            Self::Memory { .. } => true,
            #[cfg(any(test, feature = "bench"))]
            Self::Discard => true,
        }
    }

    /// Takes a chunk; only for an inline sink.
    pub(crate) fn write_inline(&mut self, chunk: &[u8]) -> Result<(), TransferError> {
        match self {
            Self::Memory { data, limit } => {
                if (data.len() + chunk.len()) as u64 > *limit {
                    return Err(TransferError::TooLarge);
                }
                data.extend_from_slice(chunk);
                Ok(())
            }
            #[cfg(any(test, feature = "bench"))]
            Self::Discard => Ok(()),
        }
    }

    /// The collected data of a memory sink.
    pub(crate) fn into_data(self) -> Option<Vec<u8>> {
        match self {
            Self::Memory { data, .. } => Some(data),
            #[cfg(any(test, feature = "bench"))]
            Self::Discard => None,
        }
    }
}
