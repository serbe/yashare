// =================================================================================
// ClientError — configuration and usage errors
// =================================================================================

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum ClientError {
    /// The provided client configuration is invalid.
    ///
    /// This can occur when configuration values are out of bounds, mutually
    /// inconsistent, or otherwise cannot be used to create a valid client.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// The maximum number of link attempts is invalid (e.g., zero).
    #[error("invalid max link attempts: {0}")]
    InvalidMaxLinkAttempts(usize),

    /// A path provided by the user or returned by the API is invalid.
    ///
    /// This can happen when a path contains illegal characters, attempts
    /// directory traversal (`..`), or is otherwise unusable as a filesystem
    /// path.
    #[error("invalid path component: {0}")]
    InvalidPath(String),

    /// The public key provided by the user is invalid.
    ///
    /// Public keys are parsed from URLs or raw hash strings. This error
    /// indicates the input could not be parsed or does not match the
    /// expected format.
    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),

    /// A public link was expected to point to a folder but points to a file.
    ///
    /// This occurs when calling a folder-only operation (like `download_all`)
    /// on a resource that is actually a file. The error includes the
    /// resource name for context.
    #[error("public link must point to a folder, got: {0}")]
    NotAFolder(String),

    /// Failed to parse a URL.
    ///
    /// This wraps `url::ParseError` and covers invalid URLs provided by
    /// the user or constructed by the library.
    #[error("failed to parse URL: {0}")]
    UrlParse(#[from] url::ParseError),
}
