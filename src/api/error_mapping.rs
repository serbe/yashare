use std::{path::Path, time::Duration};

use reqwest::{Response, StatusCode, header::RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error,
    error::{ApiError, HttpError},
};

/// Represents an error response from the Yandex.Disk API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiErrorResponse {
    pub error: String,
    pub description: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
}

/// Maps a non-successful HTTP response from Yandex.Disk API to an `ApiError`.
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

/// Maps a download error from the HTTP response and path.
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

/// Extracts the `Retry-After` header from the response, if present.
fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}
