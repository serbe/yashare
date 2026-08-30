use std::path::PathBuf;

use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, HeaderMap, HeaderValue, RANGE},
};

use crate::{Error, error::HttpError};

/// Решение о том, как обрабатывать загрузку файла
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeAction {
    /// Начать загрузку с нуля
    Start,
    /// Продолжить с указанного смещения
    Resume(u64),
    /// Перезапустить загрузку (удалить частичный файл)
    Restart,
    /// Файл уже полностью загружен
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

/// Состояние возобновления загрузки
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

/// Управляет логикой определения состояния возобновления и работы с Range-заголовками
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResumeStateManager;

impl ResumeStateManager {
    pub(crate) fn new() -> Self {
        Self
    }

    /// Определяет состояние возобновления на основе размера существующего частичного файла
    pub(crate) fn determine_state(
        &self,
        part_path: PathBuf,
        existing_size: u64,
        expected_size: u64,
    ) -> ResumeState {
        let action = match existing_size.cmp(&expected_size) {
            _ if existing_size == 0 => ResumeAction::Start,
            std::cmp::Ordering::Greater => ResumeAction::Restart,
            std::cmp::Ordering::Equal => ResumeAction::Finalize,
            std::cmp::Ordering::Less => ResumeAction::Resume(existing_size),
        };

        ResumeState { part_path, existing_size, action }
    }

    /// Применяет Range-заголовок для возобновления загрузки
    pub(crate) fn apply_range_header(
        &self,
        headers: &mut HeaderMap,
        state: &ResumeState,
    ) -> Result<(), Error> {
        if !state.is_resuming() {
            return Ok(());
        }

        let offset = state.offset();
        let value = format!("bytes={offset}-");

        let header_value = HeaderValue::from_str(&value)
            .map_err(|_| Error::Http(HttpError::InvalidHeader(value)))?;

        headers.insert(RANGE, header_value);

        Ok(())
    }

    /// Валидирует ответ сервера для запроса с Range-заголовком
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

                if !self.content_range_starts_at(raw, state.offset(), expected_size) {
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

    /// Проверяет, что Content-Range начинается с ожидаемого смещения
    fn content_range_starts_at(
        &self,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determine_state_missing_file() {
        let manager = ResumeStateManager::new();
        let state = manager.determine_state(PathBuf::from("f.part"), 0, 100);
        assert_eq!(state.action, ResumeAction::Start);
        assert!(state.needs_download());
    }

    #[test]
    fn determine_state_partial_file() {
        let manager = ResumeStateManager::new();
        let state = manager.determine_state(PathBuf::from("f.part"), 5, 100);
        assert_eq!(state.action, ResumeAction::Resume(5));
        assert!(state.append());
    }

    #[test]
    fn determine_state_too_large_file() {
        let manager = ResumeStateManager::new();
        let state = manager.determine_state(PathBuf::from("f.part"), 200, 100);
        assert_eq!(state.action, ResumeAction::Restart);
        assert!(state.needs_download());
    }

    #[test]
    fn determine_state_complete_file() {
        let manager = ResumeStateManager::new();
        let state = manager.determine_state(PathBuf::from("f.part"), 100, 100);
        assert_eq!(state.action, ResumeAction::Finalize);
        assert!(!state.needs_download());
    }

    #[test]
    fn validate_content_range() {
        let manager = ResumeStateManager::new();
        assert!(manager.content_range_starts_at("bytes 5-99/100", 5, 100));
        assert!(!manager.content_range_starts_at("bytes 0-99/100", 5, 100));
        assert!(!manager.content_range_starts_at("bytes 5-99/50", 5, 100));
        assert!(!manager.content_range_starts_at("garbage", 5, 100));
    }

    #[test]
    fn apply_range_header_for_resume() {
        let manager = ResumeStateManager::new();
        let state = ResumeState {
            part_path: PathBuf::from("f.part"),
            existing_size: 5,
            action: ResumeAction::Resume(5),
        };
        let mut headers = HeaderMap::new();
        manager.apply_range_header(&mut headers, &state).unwrap();
        assert_eq!(headers.get(RANGE).unwrap(), "bytes=5-");
    }

    #[test]
    fn apply_range_header_for_start() {
        let manager = ResumeStateManager::new();
        let state = ResumeState {
            part_path: PathBuf::from("f.part"),
            existing_size: 0,
            action: ResumeAction::Start,
        };
        let mut headers = HeaderMap::new();
        manager.apply_range_header(&mut headers, &state).unwrap();
        assert!(headers.get(RANGE).is_none());
    }
}
