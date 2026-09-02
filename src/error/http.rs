// =================================================================================
// HttpError — transport-level failures
// =================================================================================

use std::time::Duration;

use reqwest::StatusCode;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum HttpError {
    /// The HTTP request failed at the transport layer.
    ///
    /// This includes connection failures, TLS errors, timeouts, and other
    /// low-level networking issues that occur before or during the request.
    #[error("HTTP request failed: {0}")]
    Request(#[source] reqwest::Error),

    /// The server returned `429 Too Many Requests`.
    ///
    /// This indicates rate limiting. The `retry_after` field contains the
    /// duration suggested by the server's `Retry-After` header, if present.
    #[error("request was rate limited (HTTP 429) {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },

    /// An HTTP header value was invalid and could not be used in a request.
    ///
    /// This typically happens when constructing a `Range` header with an
    /// invalid value, or when a header from an external source cannot be
    /// parsed.
    #[error("invalid HTTP header value: {0}")]
    InvalidHeader(String),

    /// Failed to create the HTTP client.
    ///
    /// This can occur if the client builder is misconfigured — for example,
    /// if an invalid TLS configuration is supplied.
    #[error("failed to create HTTP client")]
    CreateClient,

    /// The HTTP response body stream was interrupted.
    ///
    /// This wraps the underlying `reqwest` error and indicates that the
    /// connection was closed or reset while reading the response body.
    #[error("response stream interrupted")]
    StreamInterrupted(#[source] reqwest::Error),

    /// The HTTP response body read was interrupted.
    ///
    /// Similar to `StreamInterrupted`, but occurs during a blocking body
    /// read rather than while streaming.
    #[error("response body interrupted")]
    BodyInterrupted(#[source] reqwest::Error),

    /// The server returned `503 Service Unavailable`.
    #[error("service unavailable")]
    ServiceUnavailable,

    /// An unexpected HTTP status code was received.
    ///
    /// This is used as a fallback when no more specific error variant is
    /// applicable. It includes the raw status code for diagnostics.
    #[error("unexpected status code: {0}")]
    UnexpectedStatus(StatusCode),
}

impl HttpError {
    /// Returns `true` if the error is transient and may be retried.
    ///
    /// Transient HTTP errors include request failures, stream interruptions,
    /// and body read failures — essentially any error that could succeed on
    /// a subsequent attempt without changing the request.
    pub(crate) fn is_transient(&self) -> bool {
        matches!(
            self,
            HttpError::Request(_) | HttpError::StreamInterrupted(_) | HttpError::BodyInterrupted(_)
        )
    }
}
