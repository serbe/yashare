use std::path::{Path, PathBuf};

use reqwest::{StatusCode, header::HeaderMap};

use crate::{
    Error,
    download::{
        ResumeState,
        resume::{ResumeFileManager, ResumeStateManager},
    },
};

/// Фасад для управления возобновлением загрузки.
/// Объединяет логику состояния и файловые операции.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResumeManager {
    state_manager: ResumeStateManager,
    file_manager: ResumeFileManager,
}

impl ResumeManager {
    pub(crate) fn new() -> Self {
        Self {
            state_manager: ResumeStateManager::new(),
            file_manager: ResumeFileManager::new(),
        }
    }

    // ===== Делегирование состояния =====

    pub(crate) fn part_path(&self, destination: &Path) -> PathBuf {
        self.file_manager.part_path(destination)
    }

    pub(crate) async fn inspect(
        &self,
        destination: &Path,
        expected_size: u64,
    ) -> Result<ResumeState, Error> {
        let part_path = self.file_manager.part_path(destination);
        let existing_size = self.file_manager.get_existing_size(&part_path).await?;

        Ok(self
            .state_manager
            .determine_state(part_path, existing_size.unwrap_or(0), expected_size))
    }

    pub(crate) fn apply_range(
        &self,
        headers: &mut HeaderMap,
        state: &ResumeState,
    ) -> Result<(), Error> {
        self.state_manager.apply_range_header(headers, state)
    }

    pub(crate) fn validate_response(
        &self,
        state: &ResumeState,
        status: StatusCode,
        headers: &HeaderMap,
        expected_size: u64,
    ) -> Result<ResumeState, Error> {
        self.state_manager.validate_response(state, status, headers, expected_size)
    }

    // ===== Делегирование файловых операций =====

    pub(crate) async fn remove_if_exists(&self, path: &Path) -> Result<(), Error> {
        self.file_manager.remove_if_exists(path).await
    }

    pub(crate) async fn reset(&self, state: &ResumeState) -> Result<(), Error> {
        self.file_manager.remove_if_exists(&state.part_path).await
    }

    pub(crate) async fn open(&self, state: &ResumeState) -> Result<tokio::fs::File, Error> {
        self.file_manager.open_for_write(state).await
    }

    pub(crate) async fn current_size(&self, state: &ResumeState) -> Result<u64, Error> {
        self.file_manager.current_size(state).await
    }

    pub(crate) async fn rename_to_destination(
        &self,
        state: &ResumeState,
        destination: &Path,
    ) -> Result<(), Error> {
        self.file_manager.rename_to_destination(state, destination).await
    }
}

#[cfg(test)]
mod tests {
    use crate::download::{ResumeAction, ResumeManager};

    #[tokio::test]
    async fn inspect_missing_file_returns_start() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let manager = ResumeManager::new();

        let state = manager.inspect(&dest, 100).await.unwrap();
        assert_eq!(state.action, ResumeAction::Start);
        assert!(state.needs_download());
    }

    #[tokio::test]
    async fn inspect_partial_returns_resume() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let part = ResumeManager::new().part_path(&dest);
        tokio::fs::write(&part, b"hello").await.unwrap();

        let manager = ResumeManager::new();
        let state = manager.inspect(&dest, 100).await.unwrap();
        assert_eq!(state.action, ResumeAction::Resume(5));
    }

    #[tokio::test]
    async fn inspect_complete_returns_finalize() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let part = ResumeManager::new().part_path(&dest);
        tokio::fs::write(&part, vec![0u8; 100]).await.unwrap();

        let manager = ResumeManager::new();
        let state = manager.inspect(&dest, 100).await.unwrap();
        assert_eq!(state.action, ResumeAction::Finalize);
        assert!(!state.needs_download());
    }
}
