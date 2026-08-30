mod api;
mod client;
mod http;

pub use api::ApiError;
pub use client::ClientError;
pub use http::HttpError;

use std::{io, path::PathBuf};

use reqwest::StatusCode;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Api(#[from] ApiError),

    #[error(transparent)]
    Http(#[from] HttpError),

    #[error(transparent)]
    Client(#[from] ClientError),

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
    // Download integrity / resume
    // -------------------------------------------------------------------------
    #[error("invalid Content-Range header: {value}")]
    InvalidContentRange { value: String },

    #[error("invalid Content-Length header: {value}")]
    InvalidContentLength { value: String },

    #[error("invalid response for resumed download: expected HTTP 206, got {status}")]
    InvalidResumeResponse { status: StatusCode },

    #[error(
        "server returned an invalid content range: expected start {expected_start}, got {actual_start}"
    )]
    UnexpectedContentRange { expected_start: u64, actual_start: u64 },

    #[error("range not satisfiable for {path}, restarting from scratch")]
    RangeNotSatisfiable { path: PathBuf },

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

    // -------------------------------------------------------------------------
    // Cancellation
    // -------------------------------------------------------------------------
    #[error("operation was cancelled")]
    Cancelled,
}

impl Error {
    pub(crate) fn is_expired_link(&self) -> bool {
        matches!(self, Error::Api(api) if api.is_expired_link())
    }

    pub(crate) fn is_range_not_satisfiable(&self) -> bool {
        matches!(self, Error::Api(api) if api.is_range_not_satisfiable())
    }
}

pub fn io_error(path: impl Into<PathBuf>, source: io::Error) -> Error {
    Error::Io { path: path.into(), source }
}

pub type Result<T> = std::result::Result<T, Error>;

// `?` в builder.rs (`Url::parse(...)?`) должен уметь конвертировать
// `url::ParseError` сразу в `Error` без двух прыжков через `ClientError`.
impl From<url::ParseError> for Error {
    fn from(err: url::ParseError) -> Self {
        Error::Client(ClientError::UrlParse(err))
    }
}
