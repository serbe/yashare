// =================================================================================
// ClientError — конфиг/входные данные/использование клиента, не про сеть
// =================================================================================

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum ClientError {
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("invalid max link attempts: {0}")]
    InvalidMaxLinkAttempts(usize),

    #[error("invalid path component: {0}")]
    InvalidPath(String),

    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),

    #[error("public link must point to a folder, got: {0}")]
    NotAFolder(String),

    #[error("failed to parse URL: {0}")]
    UrlParse(#[from] url::ParseError),
}
