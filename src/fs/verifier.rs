use std::path::Path;

use bytes::BytesMut;
use hex::encode;
use md5::{Digest, Md5};
use sha2::Sha256;
use tokio::{
    fs::{File, metadata, try_exists},
    io::AsyncReadExt,
};

use crate::fs::checksum::{ChecksumSpec, VerificationMode};

/// Verifies file integrity by checking size and/or cryptographic hashes.
///
/// `FileVerifier` uses a reusable internal buffer to avoid allocating new
/// memory for each file verification. The buffer is sized at construction
/// time and reused across all verification operations on the same verifier
/// instance.
///
/// # Performance
/// For large files (hundreds of megabytes or more), checksum verification
/// requires reading the entire file. The buffer size determines how much
/// data is read at a time, balancing syscall overhead against memory usage.
///
/// # Thread safety
/// `FileVerifier` is not `Sync` because it contains a mutable buffer. Each
/// worker should have its own verifier instance to avoid contention.
pub(crate) struct FileVerifier {
    buffer: BytesMut,
}

impl FileVerifier {
    /// Creates a new verifier with the specified internal buffer size.
    ///
    /// The buffer is allocated immediately. The buffer size should be chosen
    /// to balance throughput (larger buffers reduce syscall overhead) against
    /// memory usage (smaller buffers reduce memory pressure under high
    /// concurrency).
    pub(crate) fn new(buffer_size: usize) -> Self {
        let mut buffer = BytesMut::with_capacity(buffer_size);
        buffer.resize(buffer_size, 0);
        Self { buffer }
    }

    /// Checks whether an existing file matches the expected size and
    /// optional checksum.
    ///
    /// This is used during pre-download checks to skip already-downloaded
    /// files. The function returns `false` if the file is missing or if
    /// either the size or checksum check fails.
    ///
    /// # Mode dependence
    /// - `SizeOnly`: only the file size is checked.
    /// - `SizeAndChecksum`: both size and checksum are checked. If the size differs, the checksum
    ///   is never computed.
    ///
    /// # Errors
    /// Returns `std::io::Error` for filesystem errors, including permission
    /// issues, I/O failures, and invalid file paths.
    pub(crate) async fn file_matches(
        &mut self,
        path: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        mode: VerificationMode,
    ) -> std::io::Result<bool> {
        if !try_exists(path).await.unwrap_or(false) {
            return Ok(false);
        }

        let metadata = metadata(path).await?;

        if metadata.len() != expected_size {
            return Ok(false);
        }

        match mode {
            VerificationMode::SizeOnly => Ok(true),
            VerificationMode::SizeAndChecksum => self.verify(path, checksum).await,
        }
    }

    /// Verifies that a file matches the expected checksum.
    ///
    /// This reads the entire file and computes the specified hash. The
    /// comparison is case-insensitive, as checksums are typically provided
    /// in lowercase but may be returned uppercase by the API.
    ///
    /// # Errors
    /// Returns `std::io::Error` if the file cannot be opened or read.
    pub(crate) async fn verify(
        &mut self,
        path: &Path,
        checksum: &ChecksumSpec,
    ) -> std::io::Result<bool> {
        match checksum {
            ChecksumSpec::Md5(expected) => {
                let actual = self.hash_file::<Md5>(path).await?;
                Ok(actual.eq_ignore_ascii_case(expected))
            },
            ChecksumSpec::Sha256(expected) => {
                let actual = self.hash_file::<Sha256>(path).await?;
                Ok(actual.eq_ignore_ascii_case(expected))
            },
            ChecksumSpec::Both { md5, sha256 } => {
                let (actual_md5, actual_sha256) = self.hash_file_both(path).await?;
                Ok(actual_md5.eq_ignore_ascii_case(md5)
                    && actual_sha256.eq_ignore_ascii_case(sha256))
            },
            ChecksumSpec::None => Ok(true),
        }
    }

    /// Computes a single hash of a file using the specified digest algorithm.
    ///
    /// The file is read in chunks, and the hash is updated incrementally.
    /// This avoids loading the entire file into memory.
    async fn hash_file<D: Digest + Default>(&mut self, path: &Path) -> std::io::Result<String> {
        let mut file = File::open(path).await?;
        let mut hasher = D::default();

        loop {
            let n = file.read(&mut self.buffer[..]).await?;
            if n == 0 {
                break;
            }
            hasher.update(&self.buffer[..n]);
        }

        Ok(encode(hasher.finalize()))
    }

    /// Computes both MD5 and SHA-256 hashes of a file in a single pass.
    ///
    /// Computing both hashes simultaneously is more efficient than computing
    /// them separately, as the file is read only once. The hashes are
    /// returned as hex strings.
    async fn hash_file_both(&mut self, path: &Path) -> std::io::Result<(String, String)> {
        let mut file = File::open(path).await?;
        let mut md5 = Md5::default();
        let mut sha256 = Sha256::default();

        loop {
            let n = file.read(&mut self.buffer[..]).await?;
            if n == 0 {
                break;
            }
            let chunk = &self.buffer[..n];
            md5.update(chunk);
            sha256.update(chunk);
        }

        Ok((encode(md5.finalize()), encode(sha256.finalize())))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::fs::write;

    use super::*;

    const CHUNK_SIZE: usize = 1024 * 1024;

    #[tokio::test]
    async fn size_only_mode_skips_hashing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.bin");
        write(&path, b"hello").await.unwrap();

        let mut verifier = FileVerifier::new(CHUNK_SIZE);
        let matches = verifier
            .file_matches(&path, 5, &ChecksumSpec::None, VerificationMode::SizeOnly)
            .await
            .unwrap();

        assert!(matches);
    }

    #[tokio::test]
    async fn checksum_mode_detects_mismatch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.bin");
        write(&path, b"hello").await.unwrap();

        let mut verifier = FileVerifier::new(CHUNK_SIZE);
        let matches = verifier
            .file_matches(
                &path,
                5,
                &ChecksumSpec::Md5("deadbeef".into()),
                VerificationMode::SizeAndChecksum,
            )
            .await
            .unwrap();

        assert!(!matches);
    }
}
