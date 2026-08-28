use std::{sync::Arc, time::Duration};

use rand::{RngExt, rng};

use crate::Error;

#[derive(Debug, Clone, Copy)]
pub(crate) enum RetryDecision {
    RetryAfter(Duration),
    Abort,
}

pub(crate) type RetryCondition = Arc<dyn Fn(&Error, usize) -> RetryDecision + Send + Sync>;

#[derive(Clone)]
pub(crate) struct RetryPolicy {
    pub(crate) max_attempts: usize,
    condition: RetryCondition,
}

impl RetryPolicy {
    pub(crate) fn new(
        max_attempts: usize,
        condition: impl Fn(&Error, usize) -> RetryDecision + Send + Sync + 'static,
    ) -> Self {
        Self {
            max_attempts,
            condition: Arc::new(condition),
        }
    }

    pub(crate) fn default_conditions(max_attempts: usize) -> Self {
        Self::new(max_attempts, |error, attempt| {
            let backoff =
                exponential_backoff(attempt, Duration::from_secs(2), Duration::from_secs(60));

            match error {
                Error::Http(_) | Error::StreamInterrupted(_) | Error::BodyInterrupted(_) => {
                    RetryDecision::RetryAfter(backoff)
                },
                Error::Status { retry_after, .. } if error.is_expired_link() => {
                    let _ = retry_after;
                    RetryDecision::Abort
                },
                Error::Status { retry_after, .. } => {
                    RetryDecision::RetryAfter(retry_after.unwrap_or(backoff))
                },
                Error::Api { .. } if error.is_expired_link() => RetryDecision::Abort,
                Error::LinkExpired { .. } | Error::InvalidContentRange { .. } => {
                    RetryDecision::RetryAfter(backoff)
                },
                Error::SizeMismatch { .. } | Error::ChecksumMismatch { .. } => {
                    RetryDecision::RetryAfter(backoff)
                },
                Error::RangeNotSatisfiable { .. } => RetryDecision::RetryAfter(backoff),
                _ => RetryDecision::Abort,
            }
        })
    }

    pub(crate) fn decide(&self, error: &Error, attempt: usize) -> RetryDecision {
        (self.condition)(error, attempt)
    }
}

fn exponential_backoff(attempt: usize, base: Duration, max: Duration) -> Duration {
    let attempt = attempt.max(1) as i32;
    let jitter = rng().random_range(0.75..1.25);
    let secs = base.as_secs_f64() * 2f64.powi(attempt - 1) * jitter;
    Duration::from_secs_f64(secs.min(max.as_secs_f64()))
}
