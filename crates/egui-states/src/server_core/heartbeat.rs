//! Detection of clients that disappeared without closing the connection.
//!
//! The server pings the client every `interval` and records when it last
//! received any frame from it. A client that stays silent for `timeout` is
//! disconnected. WebSocket endpoints answer pings automatically (tungstenite
//! on native clients, the browser itself on web clients), so only a client
//! that is gone, or whose network task is stalled, goes silent.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::time::Instant;

/// Default time between two pings.
pub(crate) const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);

/// Default time a client may stay silent before it is disconnected.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// Validated heartbeat settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Heartbeat {
    pub(crate) interval: Duration,
    pub(crate) timeout: Duration,
}

impl Heartbeat {
    /// Validates heartbeat settings. A `None` timeout disables the heartbeat,
    /// and the interval is then ignored.
    pub(crate) fn new(
        interval: Duration,
        timeout: Option<Duration>,
    ) -> Result<Option<Self>, String> {
        let Some(timeout) = timeout else {
            return Ok(None);
        };
        if interval.is_zero() {
            return Err("heartbeat interval must be greater than zero".to_string());
        }
        if timeout <= interval {
            return Err(format!(
                "heartbeat timeout ({timeout:?}) must be greater than the interval ({interval:?})"
            ));
        }
        Ok(Some(Self { interval, timeout }))
    }

    /// Validates heartbeat settings given in seconds, as the Python binding
    /// passes them.
    #[cfg(feature = "python")]
    pub(crate) fn from_secs(interval: f64, timeout: Option<f64>) -> Result<Option<Self>, String> {
        let duration = |name: &str, seconds: f64| {
            Duration::try_from_secs_f64(seconds)
                .map_err(|_| format!("heartbeat {name} must be a finite number of seconds >= 0"))
        };
        let Some(timeout) = timeout else {
            return Ok(None);
        };
        Self::new(
            duration("interval", interval)?,
            Some(duration("timeout", timeout)?),
        )
    }
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self {
            interval: DEFAULT_INTERVAL,
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
    fn settings_require_a_timeout_longer_than_a_nonzero_interval() {
        let second = Duration::from_secs(1);
        assert_eq!(
            Heartbeat::new(second, Some(3 * second)),
            Ok(Some(Heartbeat {
                interval: second,
                timeout: 3 * second
            }))
        );
        assert_eq!(Heartbeat::new(Duration::ZERO, None), Ok(None));
        assert!(Heartbeat::new(Duration::ZERO, Some(second)).is_err());
        assert!(Heartbeat::new(second, Some(second)).is_err());
        assert!(Heartbeat::new(3 * second, Some(second)).is_err());
    }

    #[test]
    fn default_settings_are_valid() {
        let heartbeat = Heartbeat::default();
        assert_eq!(
            Heartbeat::new(heartbeat.interval, Some(heartbeat.timeout)),
            Ok(Some(heartbeat))
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn settings_in_seconds_reject_negative_and_non_finite_values() {
        assert_eq!(
            Heartbeat::from_secs(0.5, Some(2.0)),
            Ok(Some(Heartbeat {
                interval: Duration::from_millis(500),
                timeout: Duration::from_secs(2)
            }))
        );
        assert_eq!(Heartbeat::from_secs(f64::NAN, None), Ok(None));
        for (interval, timeout) in [(-1.0, 2.0), (1.0, f64::INFINITY), (f64::NAN, 2.0)] {
            assert!(Heartbeat::from_secs(interval, Some(timeout)).is_err());
        }
    }

    #[tokio::test]
    async fn activity_expires_only_after_a_full_silent_timeout() {
        let activity = Activity::new();
        let timeout = Duration::from_millis(200);

        // A frame received part way through restarts the silent period.
        tokio::time::sleep(Duration::from_millis(120)).await;
        activity.touch();
        let touched = Instant::now();
        activity.expired(timeout).await;
        assert!(touched.elapsed() >= timeout);
        assert!(activity.silent_for() >= timeout);
    }
}
