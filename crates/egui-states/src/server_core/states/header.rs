//! Typed headers that the server cores send in front of a state's content.

use std::fmt;

use crate::protocol::data::{HeaderBytes, HeaderUpdate};
use crate::protocol::{FastVec, check_value_size};
use crate::server_core::error::CoreError;

/// The header type of a state, if it was registered with one, and the checks
/// every header given to it passes.
#[derive(Clone, Copy)]
pub(crate) struct HeaderType {
    type_id: Option<u32>,
    /// The state kind named in errors, such as `Data`.
    kind: &'static str,
}

impl HeaderType {
    pub(crate) fn new(kind: &'static str, type_id: Option<u32>) -> Self {
        Self { type_id, kind }
    }

    /// Whether the state was registered with a header type.
    pub(crate) fn has_type(&self) -> bool {
        self.type_id.is_some()
    }

    /// Checks a header given to state `name`: the state must have a header
    /// type and the header must fit the value size limit.
    pub(crate) fn check<'a>(
        &self,
        name: &str,
        header: Option<&'a [u8]>,
    ) -> Result<Option<&'a [u8]>, String> {
        let Some(header) = header else {
            return Ok(None);
        };
        if self.type_id.is_none() {
            let kind = self.kind;
            return Err(format!("{kind} '{name}' was registered without a header"));
        }
        check_value_size(format_args!("{name} header"), header.len())?;
        Ok(Some(header))
    }

    /// Checks that a change that creates data carries a header: a state with
    /// a header type never holds data without one. `what` completes the
    /// error message, such as "to create its data".
    pub(crate) fn require(
        &self,
        name: &str,
        header: Option<&[u8]>,
        what: impl fmt::Display,
    ) -> Result<(), CoreError> {
        if self.type_id.is_some() && header.is_none() {
            let kind = self.kind;
            return Err(CoreError::invalid_argument(format!(
                "{kind} '{name}' needs a header {what}"
            )));
        }
        Ok(())
    }

    /// The wire form of a checked header, `Keep` without one. A
    /// synchronization gives the stored header, so a state with a header
    /// type always says what it is.
    pub(crate) fn change<'a>(&self, header: Option<&'a [u8]>) -> WireHeader<'a> {
        match (self.type_id, header) {
            (Some(type_id), Some(bytes)) => WireHeader::set(type_id, bytes),
            _ => WireHeader::KEEP,
        }
    }

    /// The wire description of a checked header.
    pub(crate) fn bytes(&self, header: &[u8]) -> Option<HeaderBytes> {
        self.type_id.map(|type_id| HeaderBytes {
            type_id,
            size: header.len() as u32,
        })
    }
}

/// A header change as it goes on the wire: the update and the bytes it puts
/// in front of the content.
#[derive(Clone, Copy)]
pub(crate) struct WireHeader<'a> {
    pub(crate) update: HeaderUpdate,
    pub(crate) bytes: &'a [u8],
}

impl<'a> WireHeader<'a> {
    pub(crate) const KEEP: Self = Self {
        update: HeaderUpdate::Keep,
        bytes: &[],
    };

    pub(crate) fn set(type_id: u32, bytes: &'a [u8]) -> Self {
        Self {
            update: HeaderUpdate::Set(HeaderBytes {
                type_id,
                size: bytes.len() as u32,
            }),
            bytes,
        }
    }

    /// Serializes `header`, which carries this update, followed by the
    /// header bytes.
    pub(crate) fn message(
        &self,
        header: impl FnOnce() -> Result<FastVec<32>, ()>,
    ) -> Result<FastVec<32>, String> {
        let mut message = header().map_err(|_| "Failed to serialize header".to_string())?;
        message.extend_from_slice(self.bytes);
        Ok(message)
    }
}
