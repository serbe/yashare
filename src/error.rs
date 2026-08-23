use std::{io, path::PathBuf, time::Duration};

use reqwest::StatusCode;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("public link must point to a folder, got: {0}")]
    NotAFolder(String),

    #[error("connection failed")]
    Connect(#[source] reqwest::Error),

    #[error("http status {status}")]
    Status {
        status: StatusCode,
        retry_after: Option<Duration>,
    },

    #[error("response stream interrupted")]
    StreamInterrupted(#[source] reqwest::Error),

    #[error("download link expired for {path}")]
    LinkExpired { path: String },

    #[error("invalid Content-Range for {path}: {raw}")]
    InvalidContentRange { path: PathBuf, raw: String },

    #[error("size mismatch for {path}: expected {expected}, got {actual}")]
    SizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error("checksum verification failed for {path}")]
    ChecksumMismatch { path: PathBuf },

    #[error("json error")]
    Json(#[from] serde_json::Error),

    #[error("I/O error for `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("invalid path component: {0}")]
    InvalidPath(String),

    #[error("invalid HTTP header value: {0}")]
    InvalidHeader(#[from] reqwest::header::InvalidHeaderValue),

    #[error("api request failed after retries: {0}")]
    RetriesExhausted(String),

    #[error("unexpected api response: {0}")]
    UnexpectedResponse(String),

    #[error("incomplete download: expected {expected} bytes, got {actual}")]
    Incomplete { expected: u64, actual: u64 },

    #[error("operation was cancelled")]
    Cancelled,
}

pub fn io_error(path: impl Into<PathBuf>, source: io::Error) -> Error {
    Error::Io {
        path: path.into(),
        source,
    }
}

pub type Result<T> = std::result::Result<T, Error>;
