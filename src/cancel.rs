use std::{future::Future, time::Duration};

use tokio::{select, signal::ctrl_c, spawn, time::sleep};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::Error;

/// Represents a cancellation token that can be used to cancel a task.
#[derive(Clone, Debug)]
pub struct Cancel(CancellationToken);

impl Cancel {
    /// Creates a new `Cancel` instance with a new cancellation token.
    pub fn new() -> Self {
        Self::from_token(CancellationToken::new())
    }

    /// Creates a new `Cancel` instance from the given cancellation token.
    pub fn from_token(token: CancellationToken) -> Self {
        Self(token)
    }

    /// Returns a reference to the underlying cancellation token.
    pub fn token(&self) -> &CancellationToken {
        &self.0
    }

    /// Checks if the cancellation token is cancelled and returns an error if it is.
    pub fn check(&self) -> Result<(), Error> {
        if self.0.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Races a future against the cancellation token, returning an error if the token is cancelled.
    pub async fn race<F: Future>(&self, fut: F) -> Result<F::Output, Error> {
        select! {
            biased;
            _ = self.0.cancelled() => Err(Error::Cancelled),
            value = fut => Ok(value),
        }
    }

    /// Sleeps for the given duration, returning an error if the token is cancelled.
    pub async fn sleep(&self, duration: Duration) -> Result<(), Error> {
        self.race(sleep(duration)).await
    }

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
