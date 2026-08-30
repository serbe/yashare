use std::sync::Arc;

use crate::{DownloadFailure, DownloadStats};

#[derive(Debug)]
pub struct DownloadResult {
    pub stats: Arc<DownloadStats>,
    pub failures: Vec<DownloadFailure>,
}
