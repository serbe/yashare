#[derive(Debug, Clone)]
pub enum ChecksumSpec {
    Md5(String),
    Sha256(String),
    Both { md5: String, sha256: String },
    None,
}

impl ChecksumSpec {
    pub fn from_parts(md5: Option<String>, sha256: Option<String>) -> Self {
        match (md5, sha256) {
            (Some(md5), Some(sha256)) => Self::Both { md5, sha256 },
            (Some(md5), None) => Self::Md5(md5),
            (None, Some(sha256)) => Self::Sha256(sha256),
            (None, None) => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerifyMode {
    /// Сравнивать только размер файла (быстро, но не защищает от битых данных).
    #[default]
    SizeOnly,
    /// Дополнительно проверять md5/sha256, если они известны для элемента.
    Checksum,
}
