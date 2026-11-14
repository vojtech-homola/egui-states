//! The server's file transfer endpoints.
//!
//! The endpoints are registered like states, so their ids are unique and the
//! front-ends find them by kind, but they are never synchronized or
//! acknowledged: the transfer engine carries them.

/// A registered `Download` or `Upload` endpoint.
pub(crate) struct TransferEndpoint {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
}
