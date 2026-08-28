use std::path::{Path, PathBuf};

use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, HeaderMap, HeaderValue, RANGE},
};
use tokio::fs::{OpenOptions, metadata, remove_file};

use crate::{Error, io_error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeAction {
    Start,
    Resume(u64),
    Restart,
    Finalize,
}

impl ResumeAction {
    pub(crate) fn offset(self) -> u64 {
        match self {
            Self::Resume(offset) => offset,
            Self::Start | Self::Restart | Self::Finalize => 0,
        }
    }

    pub(crate) fn append(self) -> bool {
        matches!(self, Self::Resume(_))
    }

    pub(crate) fn needs_download(self) -> bool {
        !matches!(self, Self::Finalize)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResumeState {
    pub(crate) part_path: PathBuf,
    pub(crate) existing_size: u64,
    pub(crate) action: ResumeAction,
}

impl ResumeState {
    pub(crate) fn is_resuming(&self) -> bool {
        matches!(self.action, ResumeAction::Resume(_))
    }

    pub(crate) fn offset(&self) -> u64 {
        self.action.offset()
    }

    pub(crate) fn append(&self) -> bool {
        self.action.append()
    }

    pub(crate) fn needs_download(&self) -> bool {
        self.action.needs_download()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResumeManager;

impl ResumeManager {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn part_path(destination: &Path) -> PathBuf {
        let mut path = destination.as_os_str().to_os_string();
        path.push(".part");
        PathBuf::from(path)
    }

    pub(crate) async fn inspect(
        &self,
        destination: &Path,
        expected_size: u64,
    ) -> Result<ResumeState, Error> {
        let part_path = Self::part_path(destination);

        let existing_size = match metadata(&part_path).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(io_error(&part_path, error)),
        };

        let action = match existing_size.cmp(&expected_size) {
            _ if existing_size == 0 => ResumeAction::Start,
            std::cmp::Ordering::Greater => ResumeAction::Restart,
            std::cmp::Ordering::Equal => ResumeAction::Finalize,
            std::cmp::Ordering::Less => ResumeAction::Resume(existing_size),
        };

        Ok(ResumeState { part_path, existing_size, action })
    }

    pub(crate) async fn remove_if_exists(&self, path: &Path) -> Result<(), Error> {
        match remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(path, error)),
        }
    }

    pub(crate) async fn reset(&self, state: &ResumeState) -> Result<(), Error> {
        self.remove_if_exists(&state.part_path).await
    }

    pub(crate) fn apply_range(
        &self,
        headers: &mut HeaderMap,
        state: &ResumeState,
    ) -> Result<(), Error> {
        if !state.is_resuming() {
            return Ok(());
        }

        let offset = state.offset();
        let value = format!("bytes={offset}-");

        let header_value =
            HeaderValue::from_str(&value).map_err(|_| Error::InvalidHeader(value))?;

        headers.insert(RANGE, header_value);

        Ok(())
    }

    pub(crate) fn validate_response(
        &self,
        state: &ResumeState,
        status: StatusCode,
        headers: &HeaderMap,
        expected_size: u64,
    ) -> Result<ResumeState, Error> {
        if !state.is_resuming() {
            return Ok(state.clone());
        }

        match status {
            StatusCode::PARTIAL_CONTENT => {
                let raw = headers
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("<missing>");

                if !content_range_starts_at(raw, state.offset(), expected_size) {
                    return Err(Error::InvalidContentRange { value: raw.to_string() });
                }

                Ok(state.clone())
            },

            StatusCode::OK => Ok(ResumeState {
                part_path: state.part_path.clone(),
                existing_size: 0,
                action: ResumeAction::Restart,
            }),

            _ => Ok(state.clone()),
        }
    }

    pub(crate) async fn open(&self, state: &ResumeState) -> Result<tokio::fs::File, Error> {
        let append = state.append();

        OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&state.part_path)
            .await
            .map_err(|error| io_error(&state.part_path, error))
    }

    pub(crate) async fn current_size(&self, state: &ResumeState) -> Result<u64, Error> {
        metadata(&state.part_path)
            .await
            .map(|metadata| metadata.len())
            .map_err(|error| io_error(&state.part_path, error))
    }
}

pub(crate) fn content_range_starts_at(
    header_value: &str,
    expected_start: u64,
    expected_total: u64,
) -> bool {
    let Some(rest) = header_value.strip_prefix("bytes ") else {
        return false;
    };

    let mut parts = rest.split('/');

    let (Some(range), Some(total)) = (parts.next(), parts.next()) else {
        return false;
    };

    let mut range_parts = range.split('-');

    let Some(start) = range_parts.next().and_then(|value| value.parse::<u64>().ok()) else {
        return false;
    };

    let Some(total) = total.parse::<u64>().ok() else {
        return false;
    };

    start == expected_start && total == expected_total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_file_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let manager = ResumeManager::new();

        let state = manager.inspect(&dest, 100).await.unwrap();
        assert_eq!(state.action, ResumeAction::Start);
        assert!(state.needs_download());
    }

    #[tokio::test]
    async fn partial_smaller_than_expected_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let part = ResumeManager::part_path(&dest);
        tokio::fs::write(&part, b"hello").await.unwrap();

        let manager = ResumeManager::new();
        let state = manager.inspect(&dest, 100).await.unwrap();
        assert_eq!(state.action, ResumeAction::Resume(5));
        assert!(state.append());
    }

    #[tokio::test]
    async fn partial_larger_than_expected_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let part = ResumeManager::part_path(&dest);
        tokio::fs::write(&part, vec![0u8; 200]).await.unwrap();

        let manager = ResumeManager::new();
        let state = manager.inspect(&dest, 100).await.unwrap();
        assert_eq!(state.action, ResumeAction::Restart);
        assert!(state.needs_download());
    }

    #[tokio::test]
    async fn partial_matching_expected_size_finalizes_without_download() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let part = ResumeManager::part_path(&dest);
        tokio::fs::write(&part, vec![0u8; 100]).await.unwrap();

        let manager = ResumeManager::new();
        let state = manager.inspect(&dest, 100).await.unwrap();

        assert_eq!(state.action, ResumeAction::Finalize);
        assert!(!state.needs_download(), "Finalize must skip the network request");
    }

    #[tokio::test]
    async fn reset_removes_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let part = ResumeManager::part_path(&dest);
        tokio::fs::write(&part, b"junk").await.unwrap();

        let manager = ResumeManager::new();
        let state = manager.inspect(&dest, 4).await.unwrap();
        manager.reset(&state).await.unwrap();

        assert!(!part.exists());
    }

    #[tokio::test]
    async fn reset_is_noop_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let manager = ResumeManager::new();

        let state = manager.inspect(&dest, 100).await.unwrap();
        manager.reset(&state).await.unwrap();
    }

    #[test]
    fn parses_valid_content_range() {
        assert!(content_range_starts_at("bytes 5-99/100", 5, 100));
    }

    #[test]
    fn rejects_mismatched_content_range() {
        assert!(!content_range_starts_at("bytes 0-99/100", 5, 100));
        assert!(!content_range_starts_at("bytes 5-99/50", 5, 100));
        assert!(!content_range_starts_at("garbage", 5, 100));
    }
}
