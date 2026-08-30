use std::{io, path::PathBuf, time::Duration};

use reqwest::StatusCode;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    // Некоторые возможные ошибки:

    //     400 — Некорректные данные.
    //     401 — Не авторизован.
    //     406 — Ресурс не может быть представлен в запрошенном формате.
    //     413 — Загрузка файла недоступна. Файл слишком большой.
    //     423 — Технические работы. Сейчас можно только просматривать и скачивать файлы.
    //     429 — Слишком много запросов.
    //     503 — Сервис временно недоступен.

    // -------------------------------------------------------------------------
    // HTTP / transport
    // -------------------------------------------------------------------------
    #[error("HTTP request failed: {0}")]
    Http(#[source] reqwest::Error),

    #[error("request was rate limited (HTTP 429) {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },

    #[error("invalid HTTP header value: {0}")]
    InvalidHeader(String),

    // -------------------------------------------------------------------------
    // Filesystem
    // -------------------------------------------------------------------------
    #[error("I/O error while accessing {path}: {source}")]
    Io {
        path: PathBuf,

        #[source]
        source: io::Error,
    },

    #[error("failed to create directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,

        #[source]
        source: io::Error,
    },

    #[error("failed to create file {path}: {source}")]
    CreateFile {
        path: PathBuf,

        #[source]
        source: io::Error,
    },

    #[error("failed to open file {path}: {source}")]
    OpenFile {
        path: PathBuf,

        #[source]
        source: io::Error,
    },

    #[error("failed to write file {path}: {source}")]
    WriteFile {
        path: PathBuf,

        #[source]
        source: io::Error,
    },

    #[error("failed to rename {from} to {to}: {source}")]
    RenameFile {
        from: PathBuf,
        to: PathBuf,

        #[source]
        source: io::Error,
    },

    // -------------------------------------------------------------------------
    // Resume / download
    // -------------------------------------------------------------------------
    #[error("invalid Content-Range header: {value}")]
    InvalidContentRange { value: String },

    #[error("invalid Content-Length header: {value}")]
    InvalidContentLength { value: String },

    #[error(
        "invalid response for resumed download: \
             expected HTTP 206, got {status}"
    )]
    InvalidResumeResponse { status: StatusCode },

    #[error(
        "server returned an invalid content range: \
             expected start {expected_start}, got {actual_start}"
    )]
    UnexpectedContentRange { expected_start: u64, actual_start: u64 },

    #[error("download URL is missing")]
    MissingDownloadUrl,

    #[error("download filename is missing")]
    MissingFileName,

    // -------------------------------------------------------------------------
    // API / response
    // -------------------------------------------------------------------------
    #[error("failed to deserialize API response: {0}")]
    Json(#[source] serde_json::Error),

    #[error("API response is missing required field: {field}")]
    MissingResponseField { field: &'static str },

    // -------------------------------------------------------------------------
    // Configuration / validation
    // -------------------------------------------------------------------------
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    // -------------------------------------------------------------------------
    // Cancellation
    // -------------------------------------------------------------------------
    #[error("operation was cancelled")]
    Cancelled,

    // -------------------------------------------------------------------------
    // Other
    // -------------------------------------------------------------------------
    #[error("range not satisfiable for {path}, restarting from scratch")]
    RangeNotSatisfiable { path: PathBuf },

    #[error("invalid max link attempts: {0}")]
    InvalidMaxLinkAttempts(usize),

    #[error("{error}: {message} {description}")]
    Api {
        status: StatusCode,
        error: String,
        message: String,
        description: String,
        retry_after: Option<Duration>,
    },

    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("public link must point to a folder, got: {0}")]
    NotAFolder(String),

    #[error("create client failed")]
    CreateClient,

    #[error("http status {status}")]
    Status {
        status: StatusCode,
        retry_after: Option<Duration>,
    },

    #[error("response stream interrupted")]
    StreamInterrupted(#[source] reqwest::Error),

    #[error("download link expired for {path}")]
    LinkExpired { path: String },

    #[error("size mismatch for {path}: expected {expected}, got {actual}")]
    SizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error("checksum verification failed for {path}")]
    ChecksumMismatch { path: PathBuf },

    #[error("response body interrupted")]
    BodyInterrupted(#[source] reqwest::Error),

    #[error("invalid path component: {0}")]
    InvalidPath(String),

    #[error("unexpected api response: {0}")]
    UnexpectedResponse(String),

    #[error("failed to parse URL: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),
}

impl Error {
    pub(crate) fn is_expired_link(&self) -> bool {
        matches!(
            self,
            Error::Status { status, .. } | Error::Api { status, .. }
                if matches!(status, &StatusCode::FORBIDDEN | &StatusCode::GONE)
        )
    }

    pub(crate) fn is_range_not_satisfiable(&self) -> bool {
        matches!(
            self,
            Error::Status { status, .. } | Error::Api { status, .. }
                if *status == StatusCode::RANGE_NOT_SATISFIABLE
        )
    }
}

pub fn io_error(path: impl Into<PathBuf>, source: io::Error) -> Error {
    Error::Io { path: path.into(), source }
}

pub type Result<T> = std::result::Result<T, Error>;
