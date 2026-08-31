use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Error, download::model::outcome::Outcome};

/// Statistics tracking download operations.
#[derive(Debug, Default)]
pub struct DownloadStats {
    downloaded: AtomicU64,
    resumed: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
    total: AtomicU64,

    bytes_downloaded: AtomicU64,
    bytes_resumed: AtomicU64,
    bytes_skipped: AtomicU64,
}

impl DownloadStats {
    /// Files downloaded from scratch
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    /// Files resumed from partial
    pub fn resumed(&self) -> u64 {
        self.resumed.load(Ordering::Relaxed)
    }

    /// Files already complete (skipped)
    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    /// Files that failed to download
    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    /// Total number of files to download
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Record a failed download
    pub fn record_failure(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Total bytes downloaded
    pub fn bytes_downloaded(&self) -> u64 {
        self.bytes_downloaded.load(Ordering::Relaxed)
    }

    /// Total bytes resumed from partial
    pub fn bytes_resumed(&self) -> u64 {
        self.bytes_resumed.load(Ordering::Relaxed)
    }

    /// Total bytes skipped (already complete)
    pub fn bytes_skipped(&self) -> u64 {
        self.bytes_skipped.load(Ordering::Relaxed)
    }

    /// Report download statistics
    pub fn report(&self) -> String {
        format!(
            "Downloaded: {}, Resumed: {}, Skipped: {}, Failed: {}, Total: {}, Bytes: {}",
            self.downloaded(),
            self.resumed(),
            self.skipped(),
            self.failed(),
            self.total(),
            self.bytes_downloaded()
        )
    }

    /// Record a download outcome
    #[inline]
    pub fn record(&self, outcome: Outcome, size: u64) {
        self.total.fetch_add(1, Ordering::Relaxed);

        match outcome {
            Outcome::Downloaded => {
                self.downloaded.fetch_add(1, Ordering::Relaxed);
                self.bytes_downloaded.fetch_add(size, Ordering::Relaxed);
            },
            Outcome::Resumed => {
                self.resumed.fetch_add(1, Ordering::Relaxed);
                self.bytes_resumed.fetch_add(size, Ordering::Relaxed);
            },
            Outcome::AlreadyComplete => {
                self.skipped.fetch_add(1, Ordering::Relaxed);
                self.bytes_skipped.fetch_add(size, Ordering::Relaxed);
            },
        }
    }
}

/// Represents a download failure
#[derive(Debug)]
pub struct DownloadFailure {
    pub path: String,
    pub error: Error,
}
