use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Error, download::model::outcome::Outcome};

/// Statistics for a download operation.
///
/// `DownloadStats` tracks both file counts and byte counts across four
/// outcome categories: downloaded from scratch, resumed from partial,
/// skipped (already complete), and failed.
///
/// # Thread safety
/// All fields use `AtomicU64` with relaxed ordering. This is sufficient
/// because individual counters are independent and there are no invariants
/// that require synchronization between counters.
///
/// # Memory
/// Each `DownloadStats` instance is typically wrapped in an `Arc` and shared
/// between a download pool and its workers. The atomic operations are
/// lock-free and impose minimal overhead.
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
    /// Number of files downloaded from scratch.
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    /// Number of files resumed from a partial download.
    pub fn resumed(&self) -> u64 {
        self.resumed.load(Ordering::Relaxed)
    }

    /// Number of files that were already complete and skipped.
    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    /// Number of files that failed to download.
    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    /// Total number of files in the download set.
    ///
    /// This is the sum of `downloaded + resumed + skipped + failed`
    /// (assuming no failures occurred) and is incremented for every file
    /// processed, regardless of outcome.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Records a failure for the current file.
    ///
    /// This is called when a download cannot be completed even after all
    /// retry attempts are exhausted.
    pub fn record_failure(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Total bytes downloaded from scratch.
    pub fn bytes_downloaded(&self) -> u64 {
        self.bytes_downloaded.load(Ordering::Relaxed)
    }

    /// Total bytes resumed from partial downloads.
    pub fn bytes_resumed(&self) -> u64 {
        self.bytes_resumed.load(Ordering::Relaxed)
    }

    /// Total bytes skipped (already complete).
    pub fn bytes_skipped(&self) -> u64 {
        self.bytes_skipped.load(Ordering::Relaxed)
    }

    /// Returns a human-readable summary of the statistics.
    ///
    /// This is primarily useful for logging and debugging.
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

    /// Records the outcome of a completed file download.
    ///
    /// This updates both the file counter and the byte counter for the
    /// corresponding outcome category. The `total` counter is incremented
    /// for every file, regardless of outcome.
    ///
    /// # Arguments
    /// - `outcome`: The outcome of the download.
    /// - `size`: The size of the file in bytes.
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

/// Represents a file that could not be downloaded.
///
/// This is included in `DownloadResult::failures` and contains enough
/// information to identify the failed file and understand why it failed.
#[derive(Debug)]
pub struct DownloadFailure {
    pub path: String,
    pub error: Error,
}
