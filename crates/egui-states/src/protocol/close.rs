//! The WebSocket close codes the server ends a connection with.
//!
//! They tell the client why it was refused or disconnected. The codes are in
//! the range WebSocket reserves for applications (4000-4999), so a client of
//! another protocol version still reads them.

/// The client speaks another protocol version.
pub(crate) const PROTOCOL_MISMATCH: u16 = 4000;
/// The client's application version is missing or differs.
pub(crate) const VERSION_MISMATCH: u16 = 4001;
/// The client's token is missing or wrong.
pub(crate) const TOKEN_REJECTED: u16 = 4002;
/// Another client is connected and the server keeps it.
#[cfg_attr(not(feature = "client"), allow(dead_code))]
pub(crate) const BUSY: u16 = 4003;
/// The server's connect-request callback refused the client.
#[cfg_attr(not(feature = "client"), allow(dead_code))]
pub(crate) const REFUSED: u16 = 4004;
/// A new client took over the connection.
pub(crate) const REPLACED: u16 = 4005;
/// The server ended the connection: it disconnected the client, stopped, or
/// failed to synchronize it.
pub(crate) const CLOSED_BY_SERVER: u16 = 4006;

#[cfg(any(feature = "server", feature = "python"))]
/// Longest close reason a WebSocket close frame can carry, in bytes.
const MAX_REASON: usize = 123;

#[cfg(any(feature = "server", feature = "python"))]
/// Why the server closes a connection: a close code and a readable reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Close {
    pub(crate) code: u16,
    pub(crate) reason: String,
}

#[cfg(any(feature = "server", feature = "python"))]
impl Close {
    /// Creates a close with `reason` cut to what a close frame can carry, at a
    /// character boundary.
    pub(crate) fn new(code: u16, reason: impl Into<String>) -> Self {
        let mut reason = reason.into();
        if reason.len() > MAX_REASON {
            let mut end = MAX_REASON;
            while !reason.is_char_boundary(end) {
                end -= 1;
            }
            reason.truncate(end);
        }
        Self { code, reason }
    }
}

#[cfg(all(test, any(feature = "server", feature = "python")))]
mod tests {
    use super::*;

    #[test]
    fn long_reasons_are_cut_at_a_character_boundary() {
        let close = Close::new(BUSY, "é".repeat(100));
        assert!(close.reason.len() <= MAX_REASON);
        assert_eq!(close.reason.len(), 122);
        assert_eq!(Close::new(BUSY, "short").reason, "short");
    }
}
