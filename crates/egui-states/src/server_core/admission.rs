//! Which client the server lets in when one is already connected.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::oneshot;

/// How long a connect-request callback may take by default.
pub(crate) const DEFAULT_CONNECT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

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

/// A client that passed the handshake and waits for the connect-request
/// callback to decide on it.
///
/// The Rust server hands it to its callback as `ConnectRequest`, the Python
/// server as its `ConnectRequest` class.
pub(crate) struct Request {
    address: String,
    pending: AtomicBool,
    reply: Mutex<Option<oneshot::Sender<bool>>>,
}

impl Request {
    /// A pending request for the client at `address`, and the receiver of
    /// its answer.
    pub(crate) fn new(address: String) -> (Arc<Self>, oneshot::Receiver<bool>) {
        let (reply, answer) = oneshot::channel();
        let request = Self {
            address,
            pending: AtomicBool::new(true),
            reply: Mutex::new(Some(reply)),
        };
        (Arc::new(request), answer)
    }

    /// The client's address.
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// Whether the server still waits for the answer.
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    /// Answers the request; only the first answer to a pending request counts.
    pub(crate) fn answer(&self, accept: bool) {
        let reply = self.reply.lock().take();
        self.pending.store(false, Ordering::Release);
        if let Some(reply) = reply {
            let _ = reply.send(accept);
        }
    }

    /// Ends the request without an answer: the client left, the time ran out,
    /// or the server stopped. A later answer is ignored.
    pub(crate) fn end(&self) {
        self.pending.store(false, Ordering::Release);
        self.reply.lock().take();
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;

    #[test]
    fn only_the_first_answer_to_a_pending_request_counts() {
        let (request, mut answer) = Request::new("peer".to_string());
        assert!(request.is_pending());
        assert_eq!(request.address(), "peer");
        request.answer(false);
        request.answer(true);
        assert!(!request.is_pending());
        assert_eq!(answer.try_recv(), Ok(false));

        let (request, mut answer) = Request::new("peer".to_string());
        request.end();
        request.answer(true);
        assert!(!request.is_pending());
        assert!(answer.try_recv().is_err(), "an ended request was answered");
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
