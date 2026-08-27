use std::path::{Path, PathBuf};

pub fn to_part_path(destination: &Path) -> PathBuf {
    let mut path = destination.as_os_str().to_os_string();
    path.push(".part");
    PathBuf::from(path)
}

pub fn content_range_starts_at(
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
    let Some(start) = range_parts.next().and_then(|s| s.parse::<u64>().ok()) else {
        return false;
    };
    let Some(total) = total.parse::<u64>().ok() else {
        return false;
    };
    start == expected_start && total == expected_total
}
