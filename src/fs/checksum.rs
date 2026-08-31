/// Specifies which checksums to verify.
#[derive(Debug, Clone)]
pub enum ChecksumSpec {
    Md5(String),
    Sha256(String),
    Both { md5: String, sha256: String },
    None,
}

impl ChecksumSpec {
    /// Creates a `ChecksumSpec` from the given MD5 and SHA-256 checksum strings.
    pub fn from_parts(md5: Option<String>, sha256: Option<String>) -> Self {
        match (md5, sha256) {
            (Some(md5), Some(sha256)) => Self::Both { md5, sha256 },
            (Some(md5), None) => Self::Md5(md5),
            (None, Some(sha256)) => Self::Sha256(sha256),
            (None, None) => Self::None,
        }
    }
}

/// Controls file integrity verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerificationMode {
    #[default]
    SizeOnly,
    SizeAndChecksum,
}
