// =================================================================================
// ApiError — server responded with a non-successful response
// =================================================================================

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::Value;

use crate::retry::RetryDecision;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum ApiError {
    /// Yandex.Disk's specific `DiskNotFoundError` code.
    ///
    /// This indicates that the requested resource does not exist at the
    /// provided public key or path. It is surfaced as its own variant
    /// because it's a semantic application error rather than a generic
    /// server error, and callers may want to handle it specially.
    #[error("resource not found: {description}")]
    NotFound { description: String },

    /// The server returned an error with a structured JSON payload.
    ///
    /// This variant is used when the error response body can be parsed as
    /// `ApiErrorResponse`, providing detailed error codes and messages
    /// from the Yandex.Disk API.
    #[error("{error}: {message} {description}")]
    Response {
        status: StatusCode,
        error: String,
        message: String,
        description: String,
        details: Option<Value>,
        retry_after: Option<Duration>,
    },

    /// The server returned an error status, but the response body could not
    /// be parsed.
    ///
    /// This can happen when a proxy or load balancer returns an HTML error
    /// page instead of the expected JSON, or when the API response format
    /// changes unexpectedly.
    #[error("http status {status}")]
    Status {
        status: StatusCode,
        retry_after: Option<Duration>,
    },

    /// The HTTP request succeeded, but the response body could not be
    /// deserialized into the expected structure.
    ///
    /// This typically indicates a mismatch between the requested field set
    /// (see `ResourceField`) and the actual API response, or a change in the
    /// API's response format.
    #[error("failed to deserialize API response: {0}")]
    MalformedResponse(#[source] serde_json::Error),

    /// The response was parsed successfully but a required field was missing.
    ///
    /// This is more specific than `MalformedResponse` — the JSON structure
    /// was valid, but the expected data was absent. This can happen when
    /// the requested field set doesn't include a field that the caller later
    /// assumes exists.
    #[error("API response is missing required field: {field}")]
    MissingField { field: &'static str },

    /// The API returned a structurally valid response that was semantically
    /// unexpected.
    ///
    /// This is a catch-all for responses that don't fit any other category
    /// but are still considered invalid by the application logic.
    #[error("unexpected api response: {0}")]
    UnexpectedResponse(String),

    /// The response did not contain a download URL where one was expected.
    ///
    /// This can happen when requesting a download link for a resource that
    /// does not have an accessible file URL (e.g., a folder).
    #[error("download URL is missing")]
    MissingDownloadUrl,

    /// The response did not contain a filename where one was expected.
    #[error("download filename is missing")]
    MissingFileName,
}

impl ApiError {
    /// Returns the HTTP status code associated with this error, if any.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            ApiError::Response { status, .. } | ApiError::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Returns the `Retry-After` duration suggested by the server, if any.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            ApiError::Response { retry_after, .. } | ApiError::Status { retry_after, .. } => {
                *retry_after
            },
            _ => None,
        }
    }

    /// Returns `true` if the error indicates a download link has expired.
    ///
    /// Yandex.Disk download links are short-lived and return `403 Forbidden`
    /// or `410 Gone` when expired. This check is used by the retry logic to
    /// trigger a link refresh.
    pub(crate) fn is_expired_link(&self) -> bool {
        matches!(self.status(), Some(StatusCode::FORBIDDEN) | Some(StatusCode::GONE))
    }

    /// Returns `true` if the error indicates a `Range` request could not be
    /// satisfied.
    ///
    /// This typically means the partial file on disk is larger than the
    /// remote file, so the server cannot satisfy the requested byte range.
    pub(crate) fn is_range_not_satisfiable(&self) -> bool {
        self.status() == Some(StatusCode::RANGE_NOT_SATISFIABLE)
    }

    /// Returns `true` if the error is transient and retryable.
    ///
    /// Transient errors include rate limiting (429), service unavailability
    /// (503), and resource locking (423). These are considered recoverable
    /// by waiting and retrying.
    fn is_transient(&self) -> bool {
        matches!(
            self.status(),
            Some(StatusCode::TOO_MANY_REQUESTS)
                | Some(StatusCode::SERVICE_UNAVAILABLE)
                | Some(StatusCode::LOCKED) // 423
        )
    }

    /// Decides whether to retry a failed API request.
    ///
    /// This is the single source of truth for retry decisions based on
    /// Yandex.Disk API responses. Transient errors (429, 503, 423) are
    /// retried after the suggested or fallback backoff. Link expiry
    /// (`403`, `410`) is not retried at this level — it's handled by the
    /// link provider. All other status codes are treated as permanent
    /// failures.
    pub(crate) fn retry_decision(&self, backoff: Duration) -> RetryDecision {
        if self.is_expired_link() {
            return RetryDecision::Abort;
        }
        if self.is_transient() {
            return RetryDecision::RetryAfter(self.retry_after().unwrap_or(backoff));
        }
        RetryDecision::Abort
    }
}
