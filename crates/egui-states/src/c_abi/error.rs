//! Error type shared by every C ABI operation.

use std::fmt;

/// Failure category of a C ABI operation.
///
/// Each variant maps to one `EGUI_STATES_ERROR_*` status code with the same
/// numeric value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    /// A null pointer, unknown flag, invalid enum value, malformed string, or
    /// malformed value data.
    InvalidArgument = 1,
    /// The server is in the wrong lifecycle phase for the operation.
    InvalidState = 2,
    /// An unknown state, type name, map key, image index, or previous value.
    NotFound = 3,
    /// Written or read data does not match the state's type, or an id refers
    /// to a different kind of state.
    TypeMismatch = 4,
    /// An index, range, size limit, or image rectangle is out of bounds.
    OutOfRange = 5,
    /// A caller-provided output buffer is too small.
    BufferTooSmall = 6,
    /// No event arrived before the deadline.
    Timeout = 7,
    /// Event delivery was closed.
    Closed = 8,
    /// A state or type definition with the same name already exists.
    AlreadyExists = 9,
    /// The server could not bind or start its network thread.
    Io = 10,
    /// An unexpected internal failure.
    Internal = 11,
}

/// Error returned by C ABI operations: a category and a human-readable message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Failure category.
    pub code: ErrorCode,
    /// Human-readable description.
    pub message: String,
}

impl Error {
    /// Creates an error from a category and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub(crate) fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidState, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub(crate) fn type_mismatch(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::TypeMismatch, message)
    }

    pub(crate) fn out_of_range(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::OutOfRange, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Result type returned by C ABI operations.
pub type Result<T> = std::result::Result<T, Error>;
