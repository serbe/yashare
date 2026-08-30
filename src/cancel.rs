use std::{future::Future, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::Error;

#[derive(Clone, Debug)]
pub struct Cancel(CancellationToken);

impl Cancel {
    pub fn new(token: CancellationToken) -> Self {
        Self(token)
    }

    pub fn token(&self) -> &CancellationToken {
        &self.0
    }

    pub fn check(&self) -> Result<(), Error> {
        if self.0.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub async fn race<F: Future>(&self, fut: F) -> Result<F::Output, Error> {
        tokio::select! {
            biased;
            _ = self.0.cancelled() => Err(Error::Cancelled),
            value = fut => Ok(value),
        }
    }

    pub async fn sleep(&self, duration: Duration) -> Result<(), Error> {
        self.race(tokio::time::sleep(duration)).await
    }
}
