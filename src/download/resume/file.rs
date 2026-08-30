use std::path::{Path, PathBuf};

use tokio::fs::{File, OpenOptions, metadata, remove_file};

use crate::{Error, download::ResumeState, io_error};

/// Управляет файловыми операциями для возобновления загрузки
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResumeFileManager;

impl ResumeFileManager {
    pub(crate) fn new() -> Self {
        Self
    }

    /// Возвращает путь к частичному файлу
    pub(crate) fn part_path(&self, destination: &Path) -> PathBuf {
        let mut path = destination.as_os_str().to_os_string();
        path.push(".part");
        PathBuf::from(path)
    }

    /// Получает размер существующего частичного файла
    pub(crate) async fn get_existing_size(&self, path: &Path) -> Result<Option<u64>, Error> {
        match metadata(path).await {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(path, error)),
        }
    }

    /// Удаляет частичный файл, если он существует
    pub(crate) async fn remove_if_exists(&self, path: &Path) -> Result<(), Error> {
        match remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(path, error)),
        }
    }

    /// Открывает файл для записи с учетом состояния возобновления
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

    /// Получает текущий размер частичного файла
    pub(crate) async fn current_size(&self, state: &ResumeState) -> Result<u64, Error> {
        metadata(&state.part_path)
            .await
            .map(|metadata| metadata.len())
            .map_err(|error| io_error(&state.part_path, error))
    }

    /// Переименовывает частичный файл в целевой
    pub(crate) async fn rename_to_destination(
        &self,
        state: &ResumeState,
        destination: &Path,
    ) -> Result<(), Error> {
        tokio::fs::rename(&state.part_path, destination)
            .await
            .map_err(|error| io_error(destination, error))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.part");
        let manager = ResumeFileManager::new();
        let size = manager.get_existing_size(&path).await.unwrap();
        assert_eq!(size, None);
    }

    #[tokio::test]
    async fn get_existing_size_returns_size_for_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.part");
        tokio::fs::write(&path, b"hello").await.unwrap();
        let manager = ResumeFileManager::new();
        let size = manager.get_existing_size(&path).await.unwrap();
        assert_eq!(size, Some(5));
    }

    #[tokio::test]
    async fn remove_if_exists_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.part");
        tokio::fs::write(&path, b"hello").await.unwrap();
        let manager = ResumeFileManager::new();
        manager.remove_if_exists(&path).await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn remove_if_exists_is_noop_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.part");
        let manager = ResumeFileManager::new();
        manager.remove_if_exists(&path).await.unwrap();
    }

    #[tokio::test]
    async fn open_for_write_with_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.part");
        let state = ResumeState {
            part_path: path.clone(),
            existing_size: 5,
            action: ResumeAction::Resume(5),
        };
        let manager = ResumeFileManager::new();
        let mut file = manager.open_for_write(&state).await.unwrap();
        use tokio::io::AsyncWriteExt;
        file.write_all(b"world").await.unwrap();
        file.flush().await.unwrap();
        drop(file);
        let content = tokio::fs::read(&path).await.unwrap();
        assert_eq!(content, b"world");
    }

    #[tokio::test]
    async fn open_for_write_without_append_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.part");
        tokio::fs::write(&path, b"hello").await.unwrap();
        let state = ResumeState {
            part_path: path.clone(),
            existing_size: 5,
            action: ResumeAction::Start,
        };
        let manager = ResumeFileManager::new();
        let mut file = manager.open_for_write(&state).await.unwrap();
        use tokio::io::AsyncWriteExt;
        file.write_all(b"world").await.unwrap();
        file.flush().await.unwrap();
        drop(file);
        let content = tokio::fs::read(&path).await.unwrap();
        assert_eq!(content, b"world");
    }
}
