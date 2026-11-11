//! Size limits both peers apply.

// TODO: make these constants configurable
pub(crate) const VALUE_MAX_SIZE: usize = 1024 * 1024; // 1 MB
pub(crate) const MSG_SIZE_THRESHOLD: usize = 1024 * 1024 * 10; // 10 MB
#[cfg(any(
    all(feature = "client", not(target_arch = "wasm32")),
    feature = "server",
    feature = "python"
))]
pub(crate) const WEBSOCKET_MAX_SIZE: usize = MSG_SIZE_THRESHOLD + MSG_SIZE_THRESHOLD / 5; // 12 MB
pub(crate) const MAX_MSG_COUNT: usize = 10;

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
