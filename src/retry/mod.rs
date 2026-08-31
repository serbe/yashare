pub(crate) mod policy;

use crate::{
    Error,
    cancel::Cancel,
    retry::policy::{RetryDecision, RetryPolicy},
};

/// Represents a retryable operation that can be attempted multiple times.
pub(crate) trait Attempt {
    type Output;

    async fn attempt(&mut self, attempt_no: usize) -> Result<Self::Output, Error>;
}

/// Runs a retryable operation according to the given policy and cancellation token.
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
