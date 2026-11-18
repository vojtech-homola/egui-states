//! Which client the server lets in when one is already connected.

/// What the server does with a client that connects while another client is
/// connected.
///
/// The server serves one client at a time. A connect-request callback, when
/// registered, decides instead of the policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ClientPolicy {
    /// The new client takes over the connection. The connected client is
    /// closed and reports `CloseReason::Replaced`.
    #[default]
    Replace,
    /// The connected client keeps the connection. The new client is refused
    /// and reports `CloseReason::Busy`.
    Reject,
}

impl ClientPolicy {
    /// The policy of a number, the value of the Python `ClientPolicy`.
    #[cfg(feature = "python")]
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Replace),
            1 => Some(Self::Reject),
            _ => None,
        }
    }
}

#[cfg(all(test, feature = "python"))]
mod tests {
    use super::*;

    /// The numbers are the values of the Python `ClientPolicy` enum.
    #[test]
    fn python_codes_name_the_policies() {
        for (code, policy) in [ClientPolicy::Replace, ClientPolicy::Reject]
            .into_iter()
            .enumerate()
        {
            assert_eq!(ClientPolicy::from_code(code as u8), Some(policy));
        }
        assert_eq!(ClientPolicy::from_code(2), None);
    }
}
