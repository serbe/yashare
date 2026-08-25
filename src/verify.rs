use std::io::Result as IoResult;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use md5::Md5;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::transport::ChecksumSpec;

const CHUNK_SIZE: usize = 1024 * 1024;

pub type Verifier =
    Arc<dyn Fn(&Path) -> Pin<Box<dyn Future<Output = IoResult<bool>> + Send>> + Send + Sync>;

impl ChecksumSpec {
    pub fn from_parts(md5: Option<String>, sha256: Option<String>) -> Self {
        match (md5, sha256) {
            (Some(md5), Some(sha256)) => Self::Both { md5, sha256 },
            (Some(md5), None) => Self::Md5(md5),
            (None, Some(sha256)) => Self::Sha256(sha256),
            (None, None) => Self::SizeOnly,
        }
    }
}

async fn verify(path: &Path, checksum: &ChecksumSpec) -> IoResult<bool> {
    match checksum {
        ChecksumSpec::Md5(expected) => {
            let actual = hash_file::<Md5>(path).await?;
            Ok(actual.eq_ignore_ascii_case(expected))
        }
        ChecksumSpec::Sha256(expected) => {
            let actual = hash_file::<Sha256>(path).await?;
            Ok(actual.eq_ignore_ascii_case(expected))
        }
        ChecksumSpec::Both { md5, sha256 } => {
            let (actual_md5, actual_sha256) = hash_file_both(path).await?;
            Ok(actual_md5.eq_ignore_ascii_case(md5) && actual_sha256.eq_ignore_ascii_case(sha256))
        }
        ChecksumSpec::SizeOnly => Ok(true),
    }
}

pub async fn verifier_for(checksum: ChecksumSpec) -> Verifier {
    Arc::new(move |path: &Path| {
        let checksum = checksum.clone();
        let path = path.to_path_buf();
        Box::pin(async move { verify(&path, &checksum).await })
            as Pin<Box<dyn Future<Output = IoResult<bool>> + Send>>
    })
}

async fn hash_file<D: Digest>(path: &Path) -> IoResult<String> {
    let mut file = File::open(path).await?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

async fn hash_file_both(path: &Path) -> IoResult<(String, String)> {
    let mut file = File::open(path).await?;
    let mut md5 = Md5::new();
    let mut sha256 = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        md5.update(&buf[..n]);
        sha256.update(&buf[..n]);
    }

    Ok((hex::encode(md5.finalize()), hex::encode(sha256.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempPath};

    fn write_temp(contents: &[u8]) -> TempPath {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(contents).unwrap();
        file.into_temp_path()
    }

    #[tokio::test]
    async fn md5_matches_known_vector() {
        let path = write_temp(b"hello world");
        let checksum = ChecksumSpec::Md5("5eb63bbbe01eeed093cb22bb8f5acdc3".to_string());
        assert!(verify(&path, &checksum).await.unwrap());
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn sha256_matches_known_vector() {
        let path = write_temp(b"hello world");
        let checksum = ChecksumSpec::Sha256(
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9".to_string(),
        );
        assert!(verify(&path, &checksum).await.unwrap());
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn both_requires_both_to_match() {
        let path = write_temp(b"hello world");

        let both_correct = ChecksumSpec::Both {
            md5: "5eb63bbbe01eeed093cb22bb8f5acdc3".to_string(),
            sha256: "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9".to_string(),
        };
        assert!(verify(&path, &both_correct).await.unwrap());

        let sha_wrong = ChecksumSpec::Both {
            md5: "5eb63bbbe01eeed093cb22bb8f5acdc3".to_string(),
            sha256: "0".repeat(64).to_string(),
        };
        assert!(!verify(&path, &sha_wrong).await.unwrap());

        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn mismatch_is_detected() {
        let path = write_temp(b"hello world");
        let checksum = ChecksumSpec::Sha256("0".repeat(64));
        assert!(!verify(&path, &checksum).await.unwrap());
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn size_only_always_passes() {
        let path = write_temp(b"anything");
        assert!(verify(&path, &ChecksumSpec::SizeOnly).await.unwrap());
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn from_parts_prefers_both_when_available() {
        let checksum = ChecksumSpec::from_parts(Some("a".into()), Some("b".into()));
        assert!(matches!(checksum, ChecksumSpec::Both { .. }));

        let checksum = ChecksumSpec::from_parts(None, None);
        assert!(matches!(checksum, ChecksumSpec::SizeOnly));
    }

    #[tokio::test]
    async fn verifier_for_produces_a_working_closure() {
        let path = write_temp(b"hello world");
        let verify_fn = verifier_for(ChecksumSpec::Md5(
            "5eb63bbbe01eeed093cb22bb8f5acdc3".to_string(),
        ))
        .await;
        assert!(verify_fn(&path).await.unwrap());
        std::fs::remove_file(path).ok();
    }
}
