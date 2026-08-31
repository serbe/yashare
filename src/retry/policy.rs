use std::{sync::Arc, time::Duration};

use rand::{RngExt, rng};

use crate::Error;

/// Represents the decision to retry or abort after a failed attempt.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RetryDecision {
    RetryAfter(Duration),
    Abort,
}

/// Represents the decision to retry or abort after a failed attempt.
pub(crate) type RetryCondition = Arc<dyn Fn(&Error, usize) -> RetryDecision + Send + Sync>;

/// Represents a retry policy that determines how to handle failed attempts.
#[derive(Clone)]
pub struct RetryPolicy {
    /// The maximum number of attempts to make.
    pub(crate) max_attempts: usize,
    /// The condition function that determines whether to retry or abort.
    condition: RetryCondition,
}

impl RetryPolicy {
    /// Creates a new `RetryPolicy` with the given maximum number of attempts and condition
    /// function.
    pub(crate) fn new(
        max_attempts: usize,
        condition: impl Fn(&Error, usize) -> RetryDecision + Send + Sync + 'static,
    ) -> Self {
        Self {
            max_attempts,
            condition: Arc::new(condition),
        }
    }

    /// Returns a default `RetryPolicy` that retries up to `max_attempts` times with exponential
    /// backoff.
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

    /// Decides whether to retry or abort based on the error and attempt number.
    pub(crate) fn decide(&self, error: &Error, attempt: usize) -> RetryDecision {
        (self.condition)(error, attempt)
    }
}

/// Applies exponential backoff to the given attempt number, with a maximum duration of `max`.
fn exponential_backoff(attempt: usize, base: Duration, max: Duration) -> Duration {
    let attempt = attempt.max(1).min(30) as i32;
    let jitter = rng().random_range(0.75..1.25);
    let secs = base.as_secs_f64() * 2f64.powi(attempt - 1) * jitter;
    Duration::from_secs_f64(secs.min(max.as_secs_f64()))
}
