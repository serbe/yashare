use std::{sync::Arc, time::Duration};

use rand::{RngExt, rng};

use crate::Error;

/// Decision made by a retry policy after a failed attempt.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RetryDecision {
    /// Retry after the specified delay.
    RetryAfter(Duration),

    /// Do not retry; propagate the error.
    Abort,
}

/// Type alias for the condition function that decides whether to retry.
///
/// Takes the error and the attempt number (1-based) and returns a
/// `RetryDecision`. Must be `Send + Sync` because it is shared across
/// threads.
pub(crate) type RetryCondition = Arc<dyn Fn(&Error, usize) -> RetryDecision + Send + Sync>;

/// A retry policy with configurable maximum attempts and condition function.
///
/// # Default policy
/// The `default_conditions()` constructor provides a sensible default
/// suitable for network operations:
/// - HTTP transient errors are retried with exponential backoff.
/// - API transient errors (429, 503, 423) are retried.
/// - Link expiry is not retried (handled separately by link provider).
/// - Checksum mismatches are retried (could be transient corruption).
/// - I/O errors are not retried (considered permanent).
#[derive(Clone)]
pub struct RetryPolicy {
    /// Maximum number of attempts (including the first).
    pub(crate) max_attempts: usize,
    /// Function that decides whether to retry based on the error and attempt.
    condition: RetryCondition,
}

impl RetryPolicy {
    /// Creates a new policy with the given maximum attempts and condition.
    ///
    /// # Arguments
    /// - `max_attempts`: Maximum number of times to try the operation.
    /// - `condition`: Function that decides whether to retry.
    pub(crate) fn new(
        max_attempts: usize,
        condition: impl Fn(&Error, usize) -> RetryDecision + Send + Sync + 'static,
    ) -> Self {
        Self {
            max_attempts,
            condition: Arc::new(condition),
        }
    }

    /// Returns a default policy with exponential backoff up to `max_attempts`.
    ///
    /// The backoff is calculated as `base * 2^(attempt-1)` with jitter:
    /// - Base delay: 2 seconds
    /// - Maximum delay: 60 seconds
    /// - Jitter: ±25% random variation
    ///
    /// # Retryable errors
    /// - `Error::Http`: transient errors (request failures, stream interrupts)
    /// - `Error::Api`: transient API errors (429, 503, 423)
    /// - `Error::InvalidContentRange`: may be transient (proxy issues)
    /// - `Error::SizeMismatch` / `Error::ChecksumMismatch`: may be transient
    /// - `Error::RangeNotSatisfiable`: may be transient
    ///
    /// # Non-retryable errors
    /// - `Error::LinkExpired`: handled by link provider
    /// - `Error::Io`: filesystem errors are considered permanent
    /// - `Error::Cancelled`: abort immediately
    pub(crate) fn default_conditions(max_attempts: usize) -> Self {
        Self::new(max_attempts, |error, attempt| {
            let backoff =
                exponential_backoff(attempt, Duration::from_secs(2), Duration::from_secs(60));

            match error {
                Error::Http(http_err) if http_err.is_transient() => {
                    RetryDecision::RetryAfter(backoff)
                },
                Error::Api(api_err) => api_err.retry_decision(backoff),
                Error::LinkExpired { .. } => RetryDecision::Abort,
                Error::InvalidContentRange { .. } => RetryDecision::RetryAfter(backoff),
                Error::SizeMismatch { .. } | Error::ChecksumMismatch { .. } => {
                    RetryDecision::RetryAfter(backoff)
                },
                Error::RangeNotSatisfiable { .. } => RetryDecision::RetryAfter(backoff),
                _ => RetryDecision::Abort,
            }
        })
    }

    /// Decides whether to retry based on the error and attempt number.
    pub(crate) fn decide(&self, error: &Error, attempt: usize) -> RetryDecision {
        (self.condition)(error, attempt)
    }
}

/// Calculates exponential backoff with jitter.
///
/// # Formula
/// `base * 2^(attempt-1) * jitter`, clamped to `max`.
///
/// # Jitter
/// Random value between 0.75 and 1.25 to spread out retries and avoid
/// thundering herds.
///
/// # Bounds
/// - `attempt` is clamped to [1, 30] to prevent overflow.
/// - The result is clamped to `max`.
fn exponential_backoff(attempt: usize, base: Duration, max: Duration) -> Duration {
    let attempt = attempt.max(1).min(30) as i32;
    let jitter = rng().random_range(0.75..1.25);
    let secs = base.as_secs_f64() * 2f64.powi(attempt - 1) * jitter;
    Duration::from_secs_f64(secs.min(max.as_secs_f64()))
}
