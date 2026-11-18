//! Size limits both peers apply.

// TODO: make these constants configurable
/// The largest serialized value of a state.
pub(crate) const VALUE_MAX_SIZE: usize = 1024 * 1024; // 1 MB
/// The largest WebSocket frame and message a peer accepts.
#[cfg(any(
    all(feature = "client", not(target_arch = "wasm32")),
    feature = "server",
    feature = "python"
))]
pub(crate) const WEBSOCKET_MAX_SIZE: usize = 12 * 1024 * 1024; // 12 MB

// Messages that queue up while the writer is busy are merged into one frame,
// which saves a socket write per message. The two limits below were chosen
// with the `burst` benchmark: more than 64 messages per frame bring no
// further gain, and frames past the chunk size of bulk transfers get slower.
/// The most messages merged into one frame.
pub(crate) const MAX_MSG_COUNT: usize = 256;
/// A frame that has reached this size is sent without merging more into it.
pub(crate) const FRAME_FLUSH_SIZE: usize = 512 * 1024;

/// Check the size of a serialized value against the maximum allowed size.
///
/// Both sides of the connection use this, so the limit and the reported message
/// stay identical for values sent to the server and values sent to the client.
/// `name` is formatted only into the error.
#[inline]
pub(crate) fn check_value_size(name: impl std::fmt::Display, size: usize) -> Result<(), String> {
    if size > VALUE_MAX_SIZE {
        return Err(format!(
            "value '{}' is too large: {} bytes, max {} bytes",
            name, size, VALUE_MAX_SIZE
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_size_limit_is_inclusive() {
        assert!(check_value_size("root.value", 0).is_ok());
        assert!(check_value_size("root.value", VALUE_MAX_SIZE).is_ok());

        let error = check_value_size("root.value", VALUE_MAX_SIZE + 1).unwrap_err();
        assert!(error.contains("root.value"), "{error}");
        assert!(error.contains(&(VALUE_MAX_SIZE + 1).to_string()), "{error}");
    }
}
