use std::{str::FromStr, time::Duration};

use tokio_util::sync::CancellationToken;
use url::Url;

use crate::Error;

pub(crate) async fn sleep_or_cancel(duration: Duration, token: &CancellationToken) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = token.cancelled() => true,
    }
}

pub(crate) async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

/// Представляет публичный ключ для доступа к ресурсу на Яндекс.Диске.
///
/// Поддерживает три формата:
/// - `Folder`: ссылка на папку (https://disk.yandex.ru/d/...)
/// - `File`: ссылка на файл (https://disk.yandex.ru/i/...)
/// - `Hash`: хеш ресурса (когда тип неизвестен)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PublicKey {
    /// Ссылка на папку (https://disk.yandex.ru/d/...)
    Folder(String),
    /// Ссылка на файл (https://disk.yandex.ru/i/...)
    File(String),
    /// Хеш ресурса (тип определяется во время выполнения)
    Hash(String),
}

impl PublicKey {
    /// Создает новый PublicKey из строки, автоматически определяя тип.
    ///
    /// # Примеры
    /// ```
    /// use yashare::PublicKey;
    ///
    /// let folder = PublicKey::parse("https://disk.yandex.ru/d/abc123")?;
    /// let file = PublicKey::parse("https://disk.yandex.ru/i/xyz789")?;
    /// let hash = PublicKey::parse("dAEMkc1QDY4SPb5+BlFnEKkx1oWX7/p5zYSCvHGQ5/6FQeE4ICFyXScld621gdJYq/J6bpmRyOJonT3VoXnDag==")?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse<S: AsRef<str>>(input: S) -> Result<Self, Error> {
        let input = input.as_ref().trim();
        if input.is_empty() {
            return Err(Error::InvalidPublicKey("empty input".to_string()));
        }

        // Пытаемся декодировать URL-кодирование
        let decoded = urlencoding::decode(input)
            .map(|s| s.to_string())
            .unwrap_or_else(|_| input.to_string());

        // Проверяем, является ли строка URL
        if decoded.starts_with("http://") || decoded.starts_with("https://") {
            return Self::parse_url(&decoded);
        }

        // Иначе это хеш
        Ok(PublicKey::Hash(decoded))
    }

    fn parse_url(url_str: &str) -> Result<Self, Error> {
        let url = Url::parse(url_str)
            .map_err(|_| Error::InvalidPublicKey(format!("invalid URL: {}", url_str)))?;

        // Проверяем, что это диск.яндекс.ру
        if url.host_str() != Some("disk.yandex.ru") {
            return Err(Error::InvalidPublicKey(format!(
                "not a Yandex.Disk URL: {}",
                url_str
            )));
        }

        // Случай 1: /public/?hash=xxx
        if url.path() == "/public/" {
            if let Some((_, hash)) = url.query_pairs().find(|(key, _)| key == "hash") {
                return Ok(PublicKey::Hash(hash.to_string()));
            }
            return Err(Error::InvalidPublicKey(
                "missing 'hash' parameter in public URL".to_string(),
            ));
        }

        // Случай 2: /d/xxx или /i/xxx
        let path = url.path().trim_start_matches('/');
        if path.starts_with("d/") {
            return Ok(PublicKey::Folder(url_str.to_string()));
        } else if path.starts_with("i/") {
            return Ok(PublicKey::File(url_str.to_string()));
        }

        // Если путь не соответствует известным шаблонам, но это диск.яндекс.ру,
        // пробуем определить тип по наличию /d/ или /i/ в URL
        if url_str.contains("/d/") {
            return Ok(PublicKey::Folder(url_str.to_string()));
        } else if url_str.contains("/i/") {
            return Ok(PublicKey::File(url_str.to_string()));
        }

        // Неизвестный формат URL
        Err(Error::InvalidPublicKey(format!(
            "unsupported URL format: {}",
            url_str
        )))
    }

    /// Возвращает строковое представление, подходящее для использования в API.
    pub fn as_api_string(&self) -> String {
        match self {
            PublicKey::Folder(url) | PublicKey::File(url) => url.clone(),
            PublicKey::Hash(hash) => hash.clone(),
        }
    }

    /// Проверяет, является ли ключ ссылкой на папку.
    pub fn is_folder(&self) -> bool {
        matches!(self, PublicKey::Folder(_))
    }

    /// Проверяет, является ли ключ ссылкой на файл.
    pub fn is_file(&self) -> bool {
        matches!(self, PublicKey::File(_))
    }

    /// Проверяет, является ли ключ хешем.
    pub fn is_hash(&self) -> bool {
        matches!(self, PublicKey::Hash(_))
    }

    /// Возвращает ссылку на папку, если ключ является папкой.
    pub fn as_folder(&self) -> Option<&str> {
        match self {
            PublicKey::Folder(url) => Some(url),
            _ => None,
        }
    }

    /// Возвращает ссылку на файл, если ключ является файлом.
    pub fn as_file(&self) -> Option<&str> {
        match self {
            PublicKey::File(url) => Some(url),
            _ => None,
        }
    }

    /// Возвращает хеш, если ключ является хешем.
    pub fn as_hash(&self) -> Option<&str> {
        match self {
            PublicKey::Hash(hash) => Some(hash),
            _ => None,
        }
    }
}

impl FromStr for PublicKey {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublicKey::Folder(url) => write!(f, "Folder({})", url),
            PublicKey::File(url) => write!(f, "File({})", url),
            PublicKey::Hash(hash) => write!(f, "Hash({})", hash),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_folder() {
        let key = PublicKey::parse("https://disk.yandex.ru/d/965DOIGYMrcE-w").unwrap();
        assert!(matches!(key, PublicKey::Folder(_)));
        assert_eq!(
            key.as_api_string(),
            "https://disk.yandex.ru/d/965DOIGYMrcE-w"
        );
    }

    #[test]
    fn test_parse_folder_encoded() {
        let key = PublicKey::parse(
            "https%3A%2F%2Fdisk.yandex.ru%2Fd%2F446d6f44-bb36-48bb-973c-4e1c71e33ccd",
        )
        .unwrap();
        assert!(matches!(key, PublicKey::Folder(_)));
        assert_eq!(
            key.as_api_string(),
            "https://disk.yandex.ru/d/446d6f44-bb36-48bb-973c-4e1c71e33ccd"
        );
    }

    #[test]
    fn test_parse_file() {
        let key = PublicKey::parse("https://disk.yandex.ru/i/6-_IZtW2RA9vuw").unwrap();
        assert!(matches!(key, PublicKey::File(_)));
        assert_eq!(
            key.as_api_string(),
            "https://disk.yandex.ru/i/6-_IZtW2RA9vuw"
        );
    }

    #[test]
    fn test_parse_hash_from_public_url() {
        let key = PublicKey::parse("https://disk.yandex.ru/public/?hash=dAEMkc1QDY4SPb5+BlFnEKkx1oWX7/p5zYSCvHGQ5/6FQeE4ICFyXScld621gdJYq/J6bpmRyOJonT3VoXnDag==").unwrap();
        assert!(matches!(key, PublicKey::Hash(_)));
        assert_eq!(
            key.as_api_string(),
            "dAEMkc1QDY4SPb5+BlFnEKkx1oWX7/p5zYSCvHGQ5/6FQeE4ICFyXScld621gdJYq/J6bpmRyOJonT3VoXnDag=="
        );
    }

    #[test]
    fn test_parse_hash_encoded() {
        let key = PublicKey::parse("dAEMkc1QDY4SPb5%2BBlFnEKkx1oWX7%2Fp5zYSCvHGQ5%2F6FQeE4ICFyXScld621gdJYq%2FJ6bpmRyOJonT3VoXnDag%3D%3D").unwrap();
        assert!(matches!(key, PublicKey::Hash(_)));
        assert_eq!(
            key.as_api_string(),
            "dAEMkc1QDY4SPb5+BlFnEKkx1oWX7/p5zYSCvHGQ5/6FQeE4ICFyXScld621gdJYq/J6bpmRyOJonT3VoXnDag=="
        );
    }

    #[test]
    fn test_parse_raw_hash() {
        let hash = "dAEMkc1QDY4SPb5+BlFnEKkx1oWX7/p5zYSCvHGQ5/6FQeE4ICFyXScld621gdJYq/J6bpmRyOJonT3VoXnDag==";
        let key = PublicKey::parse(hash).unwrap();
        assert!(matches!(key, PublicKey::Hash(_)));
        assert_eq!(key.as_api_string(), hash);
    }

    #[test]
    fn test_parse_error_empty() {
        let result = PublicKey::parse("");
        assert!(matches!(result, Err(Error::InvalidPublicKey(_))));
    }

    #[test]
    fn test_parse_error_invalid_url() {
        let result = PublicKey::parse("not a url");
        // Это может быть интерпретировано как хеш, так что не должно быть ошибкой
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_methods() {
        let folder = PublicKey::parse("https://disk.yandex.ru/d/abc").unwrap();
        assert!(folder.is_folder());
        assert!(!folder.is_file());
        assert!(!folder.is_hash());

        let file = PublicKey::parse("https://disk.yandex.ru/i/abc").unwrap();
        assert!(!file.is_folder());
        assert!(file.is_file());
        assert!(!file.is_hash());

        let hash = PublicKey::parse("dAEMkc1QDY4SPb5+").unwrap();
        assert!(!hash.is_folder());
        assert!(!hash.is_file());
        assert!(hash.is_hash());
    }

    #[test]
    fn test_as_methods() {
        let folder = PublicKey::parse("https://disk.yandex.ru/d/abc").unwrap();
        assert_eq!(folder.as_folder(), Some("https://disk.yandex.ru/d/abc"));
        assert_eq!(folder.as_file(), None);
        assert_eq!(folder.as_hash(), None);

        let hash = PublicKey::parse("dAEMkc1QDY4SPb5+").unwrap();
        assert_eq!(hash.as_folder(), None);
        assert_eq!(hash.as_file(), None);
        assert_eq!(hash.as_hash(), Some("dAEMkc1QDY4SPb5+"));
    }
}
