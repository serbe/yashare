use std::{io, path::PathBuf, time::Duration};

use reqwest::StatusCode;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
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

    #[error("Url not resolved")]
    UrlNotResolved,

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

    #[error("response body interrupted")]
    BodyInterrupted(#[source] reqwest::Error),

    #[error("failed to decode JSON response: {0}")]
    Json(#[source] serde_json::Error),

    #[error("I/O error for `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("I/O error: {0}")]
    TokioIo(#[source] io::Error),

    #[error("invalid path component: {0}")]
    InvalidPath(String),

    #[error("invalid Reqwest HTTP header value: {0}")]
    InvalidReqwestHeader(#[from] reqwest::header::InvalidHeaderValue),

    #[error("invalid HTTP header value: {0}")]
    InvalidHeader(String),

    #[error("api request failed after retries: {0}")]
    RetriesExhausted(String),

    #[error("unexpected api response: {0}")]
    UnexpectedResponse(String),

    #[error("incomplete download: expected {expected} bytes, got {actual}")]
    Incomplete { expected: u64, actual: u64 },

    #[error("operation was cancelled")]
    Cancelled,

    #[error("failed to parse URL: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),
}

pub fn io_error(path: impl Into<PathBuf>, source: io::Error) -> Error {
    Error::Io {
        path: path.into(),
        source,
    }
}

pub type Result<T> = std::result::Result<T, Error>;
