mod policy;

pub(crate) use policy::{RetryDecision, RetryPolicy};

use crate::{Error, cancel::Cancel};

/// A retryable operation that can be attempted multiple times.
///
/// Implementations of `Attempt` encapsulate a single unit of work that may
/// fail transiently and be retried. The `attempt` method is called
/// repeatedly by the retry loop until it succeeds, the retry policy says to
/// abort, or the operation is cancelled.
///
/// # Thread safety
/// `attempt` is called with `&mut self`, so the implementation can maintain
/// state across retry attempts (e.g., for exponential backoff feedback, or
/// to avoid re-initializing on each attempt).
pub(crate) trait Attempt {
    /// The type of value returned on success.
    type Output;

    /// Performs one attempt of the operation.
    ///
    /// # Arguments
    /// - `attempt_no`: The 1-based attempt number. The first attempt is 1, the second is 2, etc.
    ///
    /// # Returns
    /// `Ok(Output)` on success, `Err(Error)` on failure. The retry loop
    /// will decide whether to retry based on the policy.
    async fn attempt(&mut self, attempt_no: usize) -> Result<Self::Output, Error>;
}

/// Executes a retryable operation according to a policy and cancellation token.
///
/// This is the main entry point for retry logic. It repeatedly calls
/// `op.attempt()` up to the policy's maximum attempts, applying the policy's
/// backoff between attempts.
///
/// # Behavior
/// 1. Before each attempt, checks the cancellation token.
/// 2. Calls `op.attempt()`.
/// 3. On success, returns the value immediately.
/// 4. On `Error::Cancelled`, propagates the cancellation without retrying.
/// 5. On other errors, consults the policy's `decide()` method.
///    - `RetryDecision::Abort` → returns the error immediately.
///    - `RetryDecision::RetryAfter(delay)` → sleeps for `delay` and retries.
/// 6. If all attempts are exhausted, returns the last error.
///
/// # Panics
/// This function will panic if the operation returns `Error::Cancelled` and
/// there is no last error to return after all attempts are exhausted (this
/// should never happen in practice).
pub(crate) async fn run<A: Attempt>(
    policy: &RetryPolicy,
    cancel: &Cancel,
    mut op: A,
) -> Result<A::Output, Error> {
    let mut last_error = None;

    for attempt_no in 1..=policy.max_attempts {
        cancel.check()?;

        match op.attempt(attempt_no).await {
            Ok(value) => return Ok(value),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => match policy.decide(&error, attempt_no) {
                RetryDecision::Abort => return Err(error),
                RetryDecision::RetryAfter(delay) => {
                    last_error = Some(error);
                    if attempt_no >= policy.max_attempts {
                        break;
                    }
                    cancel.sleep(delay).await?;
                },
            },
        }
    }

    Err(last_error.unwrap_or(Error::Cancelled))
}
