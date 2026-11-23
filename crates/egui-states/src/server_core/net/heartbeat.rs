//! Detection of a peer that disappeared without closing the connection.
//!
//! The server pings the client every [`Heartbeat::interval`] and records when
//! it last received any frame from it. A client that stays silent for the
//! timeout is disconnected. WebSocket endpoints answer pings automatically
//! (tungstenite on native clients, the browser itself on web clients), so only
//! a client that is gone, or whose network task is stalled, goes silent.
//!
//! The other direction works the same way: the server tells the client its
//! timeout when the connection starts, and in every interval in which it sent
//! the client nothing else it sends a `KeepAlive` message, which the page of a
//! web client sees, unlike a ping. A client that hears nothing from the server
//! for the timeout treats the connection as lost.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::time::Instant;

/// Default time a peer may stay silent before the connection counts as lost.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// The shortest and the longest time between two pings; the longest keeps
/// the timer of a huge timeout within the clock's range.
const MIN_INTERVAL: Duration = Duration::from_millis(1);
const MAX_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Validated heartbeat settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Heartbeat {
    pub(crate) timeout: Duration,
}

impl Heartbeat {
    /// Validates the heartbeat timeout. `None` disables the heartbeat.
    pub(crate) fn new(timeout: Option<Duration>) -> Result<Option<Self>, String> {
        match timeout {
            None => Ok(None),
            Some(timeout) if timeout.is_zero() => {
                Err("heartbeat timeout must be greater than zero".to_string())
            }
            Some(timeout) => Ok(Some(Self { timeout })),
        }
    }

    /// Validates a heartbeat timeout given in seconds, as the Python binding
    /// passes it.
    #[cfg(feature = "python")]
    pub(crate) fn from_secs(timeout: Option<f64>) -> Result<Option<Self>, String> {
        let Some(timeout) = timeout else {
            return Ok(None);
        };
        let timeout = Duration::try_from_secs_f64(timeout)
            .map_err(|_| "heartbeat timeout must be a finite number of seconds > 0".to_string())?;
        Self::new(Some(timeout))
    }

    /// Time between two pings, and between two checks whether the client
    /// needs a `KeepAlive`: a quarter of the timeout, so a late or lost
    /// message does not end the connection.
    pub(crate) fn interval(&self) -> Duration {
        (self.timeout / 4).clamp(MIN_INTERVAL, MAX_INTERVAL)
    }

    /// The timeout in milliseconds as the client receives it, at least 1.
    pub(crate) fn timeout_millis(&self) -> u32 {
        u32::try_from(self.timeout.as_millis())
            .unwrap_or(u32::MAX)
            .max(1)
    }
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// When a connection last received a frame from its client.
pub(crate) struct Activity {
    origin: Instant,
    /// Nanoseconds from `origin` to the latest received frame.
    last: AtomicU64,
}

impl Activity {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            origin: Instant::now(),
            last: AtomicU64::new(0),
        })
    }

    /// Records that the client was heard from now.
    pub(crate) fn touch(&self) {
        let now = self.origin.elapsed().as_nanos();
        self.last
            .fetch_max(u64::try_from(now).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Returns how long the client has been silent.
    pub(crate) fn silent_for(&self) -> Duration {
        let last = Duration::from_nanos(self.last.load(Ordering::Relaxed));
        self.origin.elapsed().saturating_sub(last)
    }

    /// Completes once the client has been silent for `timeout`.
    pub(crate) async fn expired(&self, timeout: Duration) {
        loop {
            let silent = self.silent_for();
            if silent >= timeout {
                return;
            }
            tokio::time::sleep(timeout - silent).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_require_a_nonzero_timeout() {
        let second = Duration::from_secs(1);
        assert_eq!(
            Heartbeat::new(Some(second)),
            Ok(Some(Heartbeat { timeout: second }))
        );
        assert_eq!(Heartbeat::new(None), Ok(None));
        assert!(Heartbeat::new(Some(Duration::ZERO)).is_err());
    }

    #[test]
    fn the_interval_is_a_quarter_of_the_timeout() {
        let interval = |timeout| Heartbeat { timeout }.interval();
        assert_eq!(Heartbeat::default().interval(), Duration::from_secs(5));
        assert_eq!(interval(Duration::from_secs(2)), Duration::from_millis(500));
        assert_eq!(interval(Duration::from_nanos(3)), MIN_INTERVAL);
        assert_eq!(interval(Duration::MAX), MAX_INTERVAL);
    }

    #[test]
    fn the_client_receives_the_timeout_in_milliseconds() {
        let millis = |timeout| Heartbeat { timeout }.timeout_millis();
        assert_eq!(Heartbeat::default().timeout_millis(), 20_000);
        assert_eq!(millis(Duration::from_micros(10)), 1);
        assert_eq!(millis(Duration::MAX), u32::MAX);
    }

    #[cfg(feature = "python")]
    #[test]
    fn settings_in_seconds_reject_negative_zero_and_non_finite_values() {
        assert_eq!(
            Heartbeat::from_secs(Some(2.0)),
            Ok(Some(Heartbeat {
                timeout: Duration::from_secs(2)
            }))
        );
        assert_eq!(Heartbeat::from_secs(None), Ok(None));
        for timeout in [-1.0, 0.0, f64::INFINITY, f64::NAN] {
            assert!(Heartbeat::from_secs(Some(timeout)).is_err(), "{timeout}");
        }
    }
}
