/// Specifies which checksums to verify for a downloaded file.
///
/// Yandex.Disk can provide both MD5 and SHA-256 checksums for files,
/// depending on the API response. This enum controls which one(s) to check.
///
/// # Verification semantics
/// - `Md5` or `Sha256`: check only the specified algorithm.
/// - `Both`: check both algorithms; both must match for verification to pass.
/// - `None`: skip checksum verification entirely (size-only check).
///
/// # Performance
/// Computing SHA-256 is significantly more expensive than MD5. Use `Md5`
/// or `None` for large files if performance is a concern.
#[derive(Debug, Clone)]
pub enum ChecksumSpec {
    /// Verify against an MD5 hash (32 hex characters).
    Md5(String),

    /// Verify against a SHA-256 hash (64 hex characters).
    Sha256(String),

    /// Verify against both MD5 and SHA-256 hashes.
    Both { md5: String, sha256: String },

    /// Skip checksum verification.
    None,
}

impl ChecksumSpec {
    /// Creates a `ChecksumSpec` from optional MD5 and SHA-256 checksums.
    ///
    /// If both are provided, `Both` is used. If only one is provided, the
    /// corresponding single-algorithm variant is used. If neither is
    /// provided, `None` is returned.
    pub fn from_parts(md5: Option<String>, sha256: Option<String>) -> Self {
        match (md5, sha256) {
            (Some(md5), Some(sha256)) => Self::Both { md5, sha256 },
            (Some(md5), None) => Self::Md5(md5),
            (None, Some(sha256)) => Self::Sha256(sha256),
            (None, None) => Self::None,
        }
    }
}

/// Controls how aggressively file integrity is verified.
///
/// This affects both the pre-download check (whether to skip an already-downloaded file)
/// and the post-download verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerificationMode {
    /// Only check that the file size matches the expected size.
    ///
    /// This is fast but only protects against truncated or corrupted files
    /// that happen to have the same size as the original — it will not detect
    /// bit flips or data corruption that doesn't affect the file length.
    #[default]
    SizeOnly,

    /// Check both the file size and the checksum (if available).
    ///
    /// This provides strong integrity guarantees but may be slower for large
    /// files due to the need to read the entire file and compute the checksum.
    SizeAndChecksum,
}
