//! Typed headers that the server publishes together with a state's content.

use std::fmt::{self, Display};

use serde::de::DeserializeOwned;

use crate::protocol::data::{HeaderBytes, HeaderUpdate};
use crate::protocol::{self, VALUE_MAX_SIZE, deserialize};
use crate::typed::Typed;

/// The header type of a state with a header: any `Typed` value the server
/// sends with the content it describes, published together with it.
///
/// `()` (the default) means the state has no header, and so does any custom
/// [`Typed`] type whose [`Typed::get_type`] is
/// [`ObjectType::Empty`](crate::ObjectType::Empty); such a type must decode
/// from no bytes. A state with a header type never holds content without a
/// header: the server sends one with the change that creates the content, so
/// the type needs no default.
pub trait HeaderValue: Typed + DeserializeOwned + Clone + Send + Sync + 'static {}

impl<H> HeaderValue for H where H: Typed + DeserializeOwned + Clone + Send + Sync + 'static {}

/// Names a state in header errors, such as `Data name`, without formatting
/// it unless an error happens.
#[derive(Clone, Copy)]
pub(crate) struct Label<'a>(pub(crate) &'static str, pub(crate) &'a str);

impl Display for Label<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.0, self.1)
    }
}

/// The type hash a header of `H` carries on the wire, `None` for a state
/// without a header.
pub(crate) fn header_type_id<H: HeaderValue>() -> Option<u32> {
    protocol::data::header_type_id(&H::get_type())
}

/// Splits a payload into its header bytes and the rest. The parser checked
/// that both lie inside the frame; a shorter payload is an error.
pub(crate) fn split_header(
    payload: &[u8],
    header: u32,
    label: impl Display,
) -> Result<(&[u8], &[u8]), String> {
    usize::try_from(header)
        .ok()
        .and_then(|header| payload.split_at_checked(header))
        .ok_or_else(|| format!("Header of {label} exceeds its payload"))
}

/// Decodes the header `bytes` described by `header` for the state `label`,
/// whose header type id is `type_id`.
pub(crate) fn decode_header_bytes<H: HeaderValue>(
    type_id: Option<u32>,
    header: HeaderBytes,
    bytes: &[u8],
    label: impl Display,
) -> Result<H, String> {
    let Some(type_id) = type_id else {
        return Err(format!("{label} has no header, but one was received"));
    };
    if header.size as usize > VALUE_MAX_SIZE {
        return Err(format!(
            "Header of {label} is too large: {} bytes, max {VALUE_MAX_SIZE} bytes",
            header.size
        ));
    }
    if header.type_id != type_id {
        return Err(format!("Header type id mismatch for {label}"));
    }
    deserialize(bytes).map_err(|e| format!("Header parse error: {e} for {label}"))
}

/// The header of content that a change creates: the one the change carries,
/// or for a state without a header type the value no bytes encode. A state
/// with a header type cannot create content without one.
pub(crate) fn created_header<H: HeaderValue>(
    type_id: Option<u32>,
    header: Option<H>,
    label: impl Display,
) -> Result<H, String> {
    match (header, type_id) {
        (Some(header), _) => Ok(header),
        (None, None) => {
            deserialize(&[]).map_err(|e| format!("Header type of {label} without a header: {e}"))
        }
        (None, Some(_)) => Err(format!("{label} was created without a header")),
    }
}

/// Decodes a header update; `None` keeps the current header.
pub(crate) fn decode_header<H: HeaderValue>(
    type_id: Option<u32>,
    update: HeaderUpdate,
    bytes: &[u8],
    label: impl Display,
) -> Result<Option<H>, String> {
    match update {
        HeaderUpdate::Keep => Ok(None),
        HeaderUpdate::Set(header) => decode_header_bytes(type_id, header, bytes, label).map(Some),
    }
}
