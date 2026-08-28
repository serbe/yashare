use std::{future::Future, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::Error;

#[derive(Clone, Debug)]
pub(crate) struct Cancel(CancellationToken);

impl Cancel {
    pub(crate) fn new(token: CancellationToken) -> Self {
        Self(token)
    }

    pub(crate) fn token(&self) -> &CancellationToken {
        &self.0
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.0.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) async fn race<F: Future>(&self, fut: F) -> Result<F::Output, Error> {
        tokio::select! {
            biased;
            _ = self.0.cancelled() => Err(Error::Cancelled),
            value = fut => Ok(value),
        }
    }

    pub(crate) async fn sleep(&self, duration: Duration) -> Result<(), Error> {
        self.race(tokio::time::sleep(duration)).await
    }
}
