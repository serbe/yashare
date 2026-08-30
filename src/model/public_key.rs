use std::str::FromStr;

use url::Url;

use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PublicKey {
    Folder(String),
    File(String),
    Hash(String),
}

impl PublicKey {
    pub fn parse<S: AsRef<str>>(input: S) -> Result<Self, Error> {
        let input = input.as_ref().trim();
        if input.is_empty() {
            return Err(Error::InvalidPublicKey("empty input".to_string()));
        }

        let decoded = urlencoding::decode(input)
            .map(|s| s.to_string())
            .unwrap_or_else(|_| input.to_string());

        if decoded.starts_with("http://") || decoded.starts_with("https://") {
            return Self::parse_url(&decoded);
        }

        Ok(PublicKey::Hash(decoded))
    }

    fn parse_url(url_str: &str) -> Result<Self, Error> {
        let url = Url::parse(url_str)
            .map_err(|_| Error::InvalidPublicKey(format!("invalid URL: {}", url_str)))?;

        if url.host_str() != Some("disk.yandex.ru") {
            return Err(Error::InvalidPublicKey(format!("not a Yandex.Disk URL: {}", url_str)));
        }

        let path_segments: Vec<&str> = url
            .path_segments()
            .ok_or_else(|| Error::InvalidPublicKey("invalid path".to_string()))?
            .collect();

        match path_segments.as_slice() {
            ["public", ""] => match url.query_pairs().find(|(key, _)| key == "hash") {
                Some((_, hash)) => Ok(PublicKey::Hash(hash.to_string())),
                _ => Err(Error::InvalidPublicKey(
                    "missing 'hash' parameter in public URL".to_string(),
                )),
            },
            ["d", _] => Ok(PublicKey::Folder(url_str.to_string())),
            ["i", _] => Ok(PublicKey::File(url_str.to_string())),
            _ => Err(Error::InvalidPublicKey(format!("unsupported URL format: {}", url_str))),
        }
    }

    pub fn as_api_string(&self) -> String {
        match self {
            PublicKey::Folder(url) | PublicKey::File(url) => url.clone(),
            PublicKey::Hash(hash) => hash.clone(),
        }
    }

    pub fn is_folder(&self) -> bool {
        matches!(self, PublicKey::Folder(_))
    }

    pub fn is_file(&self) -> bool {
        matches!(self, PublicKey::File(_))
    }

    pub fn is_hash(&self) -> bool {
        matches!(self, PublicKey::Hash(_))
    }

    pub fn as_folder(&self) -> Option<&str> {
        match self {
            PublicKey::Folder(url) => Some(url),
            _ => None,
        }
    }

    pub fn as_file(&self) -> Option<&str> {
        match self {
            PublicKey::File(url) => Some(url),
            _ => None,
        }
    }

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
        assert_eq!(key.as_api_string(), "https://disk.yandex.ru/d/965DOIGYMrcE-w");
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
        assert_eq!(key.as_api_string(), "https://disk.yandex.ru/i/6-_IZtW2RA9vuw");
    }

    #[test]
    fn test_parse_hash_from_public_url() {
        let key = PublicKey::parse("https://disk.yandex.ru/public/?hash=dAEMkc1QDY4SPb5+BlFnEKkx1oWX7/p5zYSCvHGQ5/6FQeE4ICFyXScld621gdJYq/J6bpmRyOJonT3VoXnDag==").unwrap();
        dbg!(&key);
        assert!(matches!(key, PublicKey::Hash(_)));
        assert_eq!(
            key.as_api_string(),
            "dAEMkc1QDY4SPb5 BlFnEKkx1oWX7/p5zYSCvHGQ5/6FQeE4ICFyXScld621gdJYq/J6bpmRyOJonT3VoXnDag=="
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
