// =================================================================================
// ApiError — сервер ответил, но ответ (статус/тело/бизнес-код) нам не подошёл
// =================================================================================

use std::time::Duration;

use reqwest::StatusCode;

use crate::retry::RetryDecision;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum ApiError {
    /// `DiskNotFoundError` — специальный код Yandex.Disk API, вынесенный отдельно,
    /// т.к. это не универсальная "серверная ошибка", а конкретный семантический случай.
    #[error("resource not found: {description}")]
    NotFound { description: String },

    /// Сервер вернул ошибку и тело успешно распарсилось как `ApiErrorResponse`.
    #[error("{error}: {message} {description}")]
    Response {
        status: StatusCode,
        error: String,
        message: String,
        description: String,
        details: Option<serde_json::Value>,
        retry_after: Option<Duration>,
    },

    /// Сервер вернул ошибку, но тело не распарсилось как `ApiErrorResponse`
    /// (например, прокси отдал HTML вместо JSON).
    #[error("http status {status}")]
    Status {
        status: StatusCode,
        retry_after: Option<Duration>,
    },

    /// HTTP был успешным, но тело не распарсилось в ожидаемую структуру ответа.
    #[error("failed to deserialize API response: {0}")]
    MalformedResponse(#[source] serde_json::Error),

    /// Ответ распарсился, но нужного поля в нём не оказалось.
    #[error("API response is missing required field: {field}")]
    MissingField { field: &'static str },

    #[error("unexpected api response: {0}")]
    UnexpectedResponse(String),

    #[error("download URL is missing")]
    MissingDownloadUrl,

    #[error("download filename is missing")]
    MissingFileName,
}

impl ApiError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            ApiError::Response { status, .. } | ApiError::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            ApiError::Response { retry_after, .. } | ApiError::Status { retry_after, .. } => {
                *retry_after
            },
            _ => None,
        }
    }

    pub(crate) fn is_expired_link(&self) -> bool {
        matches!(self.status(), Some(StatusCode::FORBIDDEN) | Some(StatusCode::GONE))
    }

    pub(crate) fn is_range_not_satisfiable(&self) -> bool {
        self.status() == Some(StatusCode::RANGE_NOT_SATISFIABLE)
    }

    fn is_transient(&self) -> bool {
        matches!(
            self.status(),
            Some(StatusCode::TOO_MANY_REQUESTS)
                | Some(StatusCode::SERVICE_UNAVAILABLE)
                | Some(StatusCode::LOCKED) // 423
        )
    }

    /// Единственное место, где решается, стоит ли повторять запрос по коду ответа
    /// Yandex.Disk API. 429/503/423 — временные; всё остальное (400/403/404/406/412...) —
    /// не лечится повтором.
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
