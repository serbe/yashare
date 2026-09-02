use std::{future::Future, time::Duration};

use tokio::{select, signal::ctrl_c, spawn, time::sleep};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::Error;

/// A cancellation token that can be used to cancel operations.
///
/// `Cancel` wraps `tokio_util::sync::CancellationToken` and provides
/// ergonomic methods for checking cancellation and racing futures against
/// the token.
///
/// # Usage
/// Clone the token and pass it to multiple tasks. When cancellation is
/// triggered (via `Cancel::spawn()` for SIGINT, or manually via the
/// underlying token), all tasks will be notified.
///
/// # Signal handling
/// The `spawn()` method sets up a background task that waits for SIGINT
/// (Ctrl+C) and cancels the token. This is called automatically by the
/// client for top-level operations.
#[derive(Clone, Debug)]
pub struct Cancel(CancellationToken);

impl Cancel {
    /// Creates a new cancellation token.
    pub fn new() -> Self {
        Self::from_token(CancellationToken::new())
    }

    /// Creates a new `Cancel` from an existing cancellation token.
    pub fn from_token(token: CancellationToken) -> Self {
        Self(token)
    }

    /// Returns a reference to the underlying cancellation token.
    pub fn token(&self) -> &CancellationToken {
        &self.0
    }

    /// Checks if the cancellation token has been triggered.
    ///
    /// # Returns
    /// `Ok(())` if the token is not cancelled, `Err(Error::Cancelled)` if it is.
    pub fn check(&self) -> Result<(), Error> {
        if self.0.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Races a future against the cancellation token.
    ///
    /// If the token is cancelled before the future completes, this returns
    /// `Error::Cancelled`. Otherwise, it returns the future's output.
    ///
    /// # Bias
    /// This uses `select! { biased; }` so the cancellation check is always
    /// evaluated before the future. This ensures prompt cancellation.
    pub async fn race<F: Future>(&self, fut: F) -> Result<F::Output, Error> {
        select! {
            biased;
            _ = self.0.cancelled() => Err(Error::Cancelled),
            value = fut => Ok(value),
        }
    }

    /// Sleeps for the given duration, returning early if cancelled.
    ///
    /// This is equivalent to `tokio::time::sleep(duration)` but with early
    /// cancellation support.
    pub async fn sleep(&self, duration: Duration) -> Result<(), Error> {
        self.race(sleep(duration)).await
    }

    /// Spawns a background task that cancels the token on SIGINT (Ctrl+C).
    ///
    /// This is typically called once at the start of a download to ensure
    /// the operation can be interrupted by the user.
    ///
    /// # Note
    /// Multiple calls to `spawn()` on the same token will spawn multiple
    /// signal handlers, but only the first one to receive SIGINT will cancel
    /// the token. Subsequent SIGINT signals are ignored by the other
    /// handlers.
    pub fn spawn(&self) -> Result<(), Error> {
        let token = self.token().clone();
        spawn(async move {
            ctrl_c().await.ok();
            warn!("⚠️  Received interrupt signal, shutting down...");
            token.cancel();
        });
        Ok(())
    }
}
