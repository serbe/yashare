mod policy;

pub(crate) use policy::{RetryDecision, RetryPolicy};

use crate::{Error, cancel::Cancel};

/// One retryable operation. Implemented per call-site instead of passed as
/// a closure, because an `FnMut(usize) -> Fut` can't express "each call
/// re-borrows `&mut self` for a fresh, non-overlapping lifetime" — the
/// closure form only works when the operation only needs `&self`.
pub(crate) trait Attempt {
    type Output;

    async fn attempt(&mut self, attempt_no: usize) -> Result<Self::Output, Error>;
}

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
