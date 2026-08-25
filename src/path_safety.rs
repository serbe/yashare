use std::path::PathBuf;

use crate::error::{Error, Result};

/// Converts a Yandex.Disk path (e.g. `/Photos/2024/img.jpg`) into a safe,
/// relative filesystem path with no `..`, empty, or reserved components.
pub fn safe_relative_path(disk_path: &str) -> Result<PathBuf> {
    let normalized = disk_path.replace('\\', "/");
    let normalized = normalized.trim_start_matches('/');

    if normalized.is_empty() {
        return Err(Error::InvalidPath("empty relative path".to_string()));
    }

    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(Error::InvalidPath(part.to_string()));
        }
        let safe_part = safe_filename_component(part);
        if safe_part.is_empty() {
            return Err(Error::InvalidPath(part.to_string()));
        }
        parts.push(safe_part);
    }

    Ok(parts.iter().collect())
}

/// Sanitizes a single filename component: strips characters invalid on
/// Windows/Unix, trailing dots/spaces, and renames reserved Windows device names.
pub fn safe_filename_component(name: &str) -> String {
    let mut result = name.trim().to_string();

    if result.is_empty() {
        return "_".to_string();
    }

    for ch in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
        result = result.replace(ch, "_");
    }

    while result.ends_with(' ') || result.ends_with('.') {
        result.pop();
    }

    if result.is_empty() {
        return "_".to_string();
    }

    let upper = result.to_uppercase();
    let reserved = matches!(
        upper.as_str(),
        "CON" | "PRN"
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
