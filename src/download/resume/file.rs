use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use tokio::fs::{File, OpenOptions, metadata, remove_file, rename};

use crate::{Error, download::ResumeState, io_error};

/// Manages file operations for resuming a download.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResumeFileManager;

impl ResumeFileManager {
    /// Creates a new `ResumeFileManager`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Returns the path to the partial file.
    pub(crate) fn part_path(&self, destination: &Path) -> PathBuf {
        let mut path = destination.as_os_str().to_os_string();
        path.push(".part");
        PathBuf::from(path)
    }

    /// Returns the size of the existing partial file, if it exists.
    pub(crate) async fn get_existing_size(&self, path: &Path) -> Result<Option<u64>, Error> {
        match metadata(path).await {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(path, error)),
        }
    }

    /// Removes the partial file if it exists.
    pub(crate) async fn remove_if_exists(&self, path: &Path) -> Result<(), Error> {
        match remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(path, error)),
        }
    }

    /// Opens the partial file for writing, creating it if it does not exist.
    pub(crate) async fn open_for_write(&self, state: &ResumeState) -> Result<File, Error> {
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

    /// Returns the current size of the partial file.
    pub(crate) async fn current_size(&self, state: &ResumeState) -> Result<u64, Error> {
        metadata(&state.part_path)
            .await
            .map(|metadata| metadata.len())
            .map_err(|error| io_error(&state.part_path, error))
    }

    /// Renames the partial file to the destination path.
    pub(crate) async fn rename_to_destination(
        &self,
        state: &ResumeState,
        destination: &Path,
    ) -> Result<(), Error> {
        rename(&state.part_path, destination)
            .await
            .map_err(|error| io_error(destination, error))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;
    use tokio::{
        fs::{read, write},
        io::AsyncWriteExt,
    };

    use crate::download::{
        ResumeState,
        resume::{ResumeAction, ResumeFileManager},
    };

    #[tokio::test]
    async fn part_path_creates_expected_path() {
        let manager = ResumeFileManager::new();
        let dest = PathBuf::from("/tmp/file.bin");
        assert_eq!(manager.part_path(&dest), PathBuf::from("/tmp/file.bin.part"));
    }

    #[tokio::test]
    async fn get_existing_size_returns_none_for_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.part");
        let manager = ResumeFileManager::new();
        let size = manager.get_existing_size(&path).await.unwrap();
        assert_eq!(size, None);
    }

    #[tokio::test]
    async fn get_existing_size_returns_size_for_existing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.part");
        write(&path, b"hello").await.unwrap();
        let manager = ResumeFileManager::new();
        let size = manager.get_existing_size(&path).await.unwrap();
        assert_eq!(size, Some(5));
    }

    #[tokio::test]
    async fn remove_if_exists_removes_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.part");
        write(&path, b"hello").await.unwrap();
        let manager = ResumeFileManager::new();
        manager.remove_if_exists(&path).await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn remove_if_exists_is_noop_for_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.part");
        let manager = ResumeFileManager::new();
        manager.remove_if_exists(&path).await.unwrap();
    }

    #[tokio::test]
    async fn open_for_write_with_append() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.part");
        write(&path, b"hello").await.unwrap();
        let state = ResumeState {
            part_path: path.clone(),
            action: ResumeAction::Resume(5),
        };
        let manager = ResumeFileManager::new();
        let mut file = manager.open_for_write(&state).await.unwrap();

        file.write_all(b"world").await.unwrap();
        file.flush().await.unwrap();
        drop(file);
        let content = read(&path).await.unwrap();
        assert_eq!(content, b"helloworld");
    }

    #[tokio::test]
    async fn open_for_write_without_append_truncates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.part");
        write(&path, b"hello").await.unwrap();
        let state = ResumeState {
            part_path: path.clone(),
            action: ResumeAction::Start,
        };
        let manager = ResumeFileManager::new();
        let mut file = manager.open_for_write(&state).await.unwrap();

        file.write_all(b"world").await.unwrap();
        file.flush().await.unwrap();
        drop(file);
        let content = read(&path).await.unwrap();
        assert_eq!(content, b"world");
    }
}
