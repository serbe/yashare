use std::{path::Path, time::Duration};

use reqwest::{Response, StatusCode, header::RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error,
    error::{ApiError, HttpError},
};

/// Error payload returned by the Yandex.Disk public API.
///
/// The Yandex.Disk API returns structured error responses with an error
/// code, human-readable message, and optional additional details.
///
/// # Example
/// ```json
/// {
///   "error": "DiskNotFoundError",
///   "description": "Resource not found",
///   "message": "The requested resource does not exist"
/// }
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiErrorResponse {
    pub error: String,
    pub description: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
}

/// Converts a non-successful API response into a structured [`ApiError`].
///
/// This function attempts to parse the response body as `ApiErrorResponse`.
/// If parsing succeeds, the structured error is returned. If parsing fails,
/// a fallback error based on the HTTP status code is returned.
///
/// # Special cases
/// - `DiskNotFoundError` is extracted as `ApiError::NotFound` with the description field, making it
///   easier for callers to handle "not found" cases specifically.
/// - The `Retry-After` header is extracted from the response (if present) and attached to the
///   error, allowing the retry policy to respect the server's suggested wait time.
///
/// # Fallback behavior
/// If the response body cannot be parsed as `ApiErrorResponse` (e.g., the
/// server returns HTML instead of JSON), the error is converted to
/// `ApiError::Status` with the status code and any `Retry-After` header.
pub(crate) async fn map_error_response(response: Response) -> ApiError {
    let status = response.status();
    let retry_after = retry_after(&response);

    match response.json::<ApiErrorResponse>().await {
        Ok(api_error) => match api_error.error.as_str() {
            "DiskNotFoundError" => ApiError::NotFound { description: api_error.description },
            _ => ApiError::Response {
                status,
                error: api_error.error,
                message: api_error.message,
                description: api_error.description,
                details: api_error.details,
                retry_after,
            },
        },
        Err(_) => ApiError::Status { status, retry_after },
    }
}

/// Converts a failed download response into a user-facing error.
///
/// This is specialized for download endpoints (as opposed to metadata APIs).
/// It translates certain HTTP status codes into semantically rich errors:
///
/// - `403 Forbidden` or `410 Gone` → `Error::LinkExpired` — the download link has expired and a
///   fresh one is needed.
/// - `416 Range Not Satisfiable` → `Error::RangeNotSatisfiable` — the partial file on disk is
///   larger than the remote file; restart from scratch.
/// - `429 Too Many Requests` → `Error::Http(HttpError::RateLimited)` — back off and retry.
/// - `503 Service Unavailable` → `Error::Http(HttpError::ServiceUnavailable)` — server is
///   temporarily overloaded.
/// - Other status codes → `Error::Http(HttpError::UnexpectedStatus)` — fallback for unexpected
///   responses.
///
/// # Arguments
/// - `response`: The HTTP response that failed.
/// - `path`: The file path being downloaded, used for context in error messages.
pub(crate) fn map_download_error(response: &Response, path: &Path) -> Error {
    let status = response.status();
    let retry_after = retry_after(response);

    match status {
        StatusCode::FORBIDDEN | StatusCode::GONE => {
            Error::LinkExpired { path: path.display().to_string() }
        },
        StatusCode::RANGE_NOT_SATISFIABLE => {
            Error::RangeNotSatisfiable { path: path.to_path_buf() }
        },
        StatusCode::TOO_MANY_REQUESTS => Error::Http(HttpError::RateLimited { retry_after }),
        StatusCode::SERVICE_UNAVAILABLE => Error::Http(HttpError::ServiceUnavailable),
        status => Error::Http(HttpError::UnexpectedStatus(status)),
    }
}

/// Extracts the `Retry-After` header as a [`Duration`].
///
/// The header may be specified as an integer number of seconds, or as an
/// HTTP date. This function only parses the integer form, which is the most
/// common for rate-limiting responses.
fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}
