// =================================================================================
// HttpError — сломался сам транспорт, до всякой семантики API
// =================================================================================

use std::time::Duration;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum HttpError {
    #[error("HTTP request failed: {0}")]
    Request(#[source] reqwest::Error),

    #[error("request was rate limited (HTTP 429) {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },

    #[error("invalid HTTP header value: {0}")]
    InvalidHeader(String),

    #[error("failed to create HTTP client")]
    CreateClient,

    #[error("response stream interrupted")]
    StreamInterrupted(#[source] reqwest::Error),

    #[error("response body interrupted")]
    BodyInterrupted(#[source] reqwest::Error),
}

impl HttpError {
    pub(crate) fn is_transient(&self) -> bool {
        matches!(
            self,
            HttpError::Request(_) | HttpError::StreamInterrupted(_) | HttpError::BodyInterrupted(_)
        )
    }
}
