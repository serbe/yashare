use std::path::PathBuf;

use crate::error::{ClientError, Error, Result};

/// Characters that are illegal in Windows filenames.
///
/// This set is used to sanitize paths from the API, which may contain
/// characters that are valid on Yandex.Disk but invalid on the local
/// filesystem.
const INVALID_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Converts a Yandex.Disk path to a safe relative filesystem path.
///
/// Yandex.Disk paths use forward slashes (`/`) as path separators and may
/// contain characters that are invalid or reserved on Windows. This function
/// normalizes the path, removes leading slashes, and sanitizes each component.
///
/// # Security
/// This function rejects paths that attempt directory traversal using `..`
/// or refer to the current directory using `.`. This prevents attackers from
/// using crafted paths to escape the intended destination directory.
///
/// # Windows compatibility
/// Reserved filenames like `CON`, `PRN`, `AUX`, etc., are prefixed with an
/// underscore to avoid conflicts. Trailing spaces and dots are stripped, as
/// Windows treats them as insignificant.
///
/// # Errors
/// Returns `ClientError::InvalidPath` if the path is empty, contains `..`
/// or `.` components, or consists entirely of invalid characters after
/// sanitization.
pub fn safe_relative_path(disk_path: &str) -> Result<PathBuf> {
    let normalized = disk_path.replace('\\', "/");
    let normalized = normalized.trim_start_matches('/');

    if normalized.is_empty() {
        return Err(Error::Client(ClientError::InvalidPath("empty relative path".to_string())));
    }

    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(Error::Client(ClientError::InvalidPath(part.to_string())));
        }
        let safe_part = safe_filename_component(part);
        if safe_part.is_empty() {
            return Err(Error::Client(ClientError::InvalidPath(part.to_string())));
        }
        parts.push(safe_part);
    }

    Ok(parts.iter().collect())
}

/// Sanitizes a single filename component.
///
/// This function:
/// - Trims leading and trailing whitespace.
/// - Replaces control characters and invalid filesystem characters with `_`.
/// - Strips trailing spaces and dots.
/// - Prefixes reserved Windows names with `_`.
///
/// # Returns
/// A sanitized filename. If the input consists entirely of invalid
/// characters or whitespace, returns `"_"` as a fallback.
pub fn safe_filename_component(name: &str) -> String {
    let mut result = name.trim().to_string();

    result = result
        .chars()
        .map(|c| {
            if c.is_control() || INVALID_CHARS.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();

    while result.ends_with(' ') || result.ends_with('.') {
        result.pop();
    }

    if result.is_empty() {
        return "_".to_string();
    }

    let upper = result.to_uppercase();
    let reserved = matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );

    if reserved {
        format!("_{result}")
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_dot_components() {
        assert!(safe_relative_path("/a/../b").is_err());
        assert!(safe_relative_path("/a/./b").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(safe_relative_path("").is_err());
        assert!(safe_relative_path("/").is_err());
    }

    #[test]
    fn sanitizes_illegal_chars() {
        assert_eq!(safe_filename_component("a:b*c"), "a_b_c");
    }

    #[test]
    fn renames_reserved_windows_names() {
        assert_eq!(safe_filename_component("CON"), "_CON");
        assert_eq!(safe_filename_component("con"), "_con");
    }

    #[test]
    fn strips_trailing_dots_and_spaces() {
        assert_eq!(safe_filename_component("name.. "), "name");
    }

    #[test]
    fn builds_nested_relative_path() {
        let p = safe_relative_path("/Photos/2024/img.jpg").unwrap();
        assert_eq!(p, PathBuf::from("Photos/2024/img.jpg"));
    }
}
