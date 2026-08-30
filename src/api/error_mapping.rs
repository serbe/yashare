use std::time::Duration;

use reqwest::{Response, header::RETRY_AFTER};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiErrorResponse {
    pub error: String,
    pub description: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

/// Переводит неуспешный HTTP-ответ Yandex.Disk API в `ApiError`.
/// Это API-специфичное знание (формат тела ошибки, коды типа
/// `DiskNotFoundError`) — оно не должно жить в универсальном HTTP-клиенте.
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

fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}
