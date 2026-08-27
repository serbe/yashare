use std::path::Path;

use bytes::BytesMut;
use md5::{Digest, Md5};
use sha2::Sha256;
use tokio::{fs::File, io::AsyncReadExt};

use crate::checksum::{ChecksumSpec, VerificationMode};

pub(crate) struct FileVerifier {
    buffer: BytesMut,
}

impl FileVerifier {
    pub(crate) fn new(buffer_size: usize) -> Self {
        let mut buffer = BytesMut::with_capacity(buffer_size);
        buffer.resize(buffer_size, 0);
        Self { buffer }
    }

    pub(crate) async fn file_matches(
        &mut self,
        path: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        mode: VerificationMode,
    ) -> std::io::Result<bool> {
        let metadata = match tokio::fs::metadata(path).await {
            Ok(m) => m,
            Err(_) => return Ok(false),
        };

        if metadata.len() != expected_size {
            return Ok(false);
        }

        match mode {
            VerificationMode::SizeOnly => Ok(true),
            VerificationMode::Checksum => self.verify(path, checksum).await,
        }
    }

    pub(crate) async fn verify(
        &mut self,
        path: &Path,
        checksum: &ChecksumSpec,
    ) -> std::io::Result<bool> {
        match checksum {
            ChecksumSpec::Md5(expected) => {
                let actual = self.hash_file::<Md5>(path).await?;
                Ok(actual.eq_ignore_ascii_case(expected))
            }
            ChecksumSpec::Sha256(expected) => {
                let actual = self.hash_file::<Sha256>(path).await?;
                Ok(actual.eq_ignore_ascii_case(expected))
            }
            ChecksumSpec::Both { md5, sha256 } => {
                let (actual_md5, actual_sha256) = self.hash_file_both(path).await?;
                Ok(actual_md5.eq_ignore_ascii_case(md5)
                    && actual_sha256.eq_ignore_ascii_case(sha256))
            }
            ChecksumSpec::None => Ok(true),
        }
    }

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

        Ok(hex::encode(hasher.finalize()))
    }

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

        Ok((hex::encode(md5.finalize()), hex::encode(sha256.finalize())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CHUNK_SIZE;

    #[tokio::test]
    async fn size_only_mode_skips_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        tokio::fs::write(&path, b"hello").await.unwrap();

        let mut verifier = FileVerifier::new(CHUNK_SIZE);
        let matches = verifier
            .file_matches(&path, 5, &ChecksumSpec::None, VerificationMode::SizeOnly)
            .await
            .unwrap();

        assert!(matches);
    }

    #[tokio::test]
    async fn checksum_mode_detects_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        tokio::fs::write(&path, b"hello").await.unwrap();

        let mut verifier = FileVerifier::new(CHUNK_SIZE);
        let matches = verifier
            .file_matches(
                &path,
                5,
                &ChecksumSpec::Md5("deadbeef".into()),
                VerificationMode::Checksum,
            )
            .await
            .unwrap();

        assert!(!matches);
    }
}
