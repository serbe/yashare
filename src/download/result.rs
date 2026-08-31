use std::sync::Arc;

use crate::{DownloadFailure, DownloadStats};

/// Represents the result of a download, including statistics and any failures that occurred.
#[derive(Debug)]
pub struct DownloadResult {
    pub stats: Arc<DownloadStats>,
    pub failures: Vec<DownloadFailure>,
}
