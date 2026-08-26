use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::{Error, download::worker::Outcome};

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
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    pub fn resumed(&self) -> u64 {
        self.resumed.load(Ordering::Relaxed)
    }

    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    pub fn record_failure(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record(&self, outcome: Outcome, size: u64) {
        self.total.fetch_add(1, Ordering::Relaxed);

        match outcome {
            Outcome::Downloaded => {
                self.downloaded.fetch_add(1, Ordering::Relaxed);
                self.bytes_downloaded.fetch_add(size, Ordering::Relaxed);
            }
            Outcome::Resumed => {
                self.resumed.fetch_add(1, Ordering::Relaxed);
                self.bytes_resumed.fetch_add(size, Ordering::Relaxed);
            }
            Outcome::AlreadyComplete => {
                self.skipped.fetch_add(1, Ordering::Relaxed);
                self.bytes_skipped.fetch_add(size, Ordering::Relaxed);
            }
        }
    }
}

/// Одна неудачная попытка — либо элемент отбракован ещё до постановки в очередь
/// (например, небезопасный путь), либо сама закачка завершилась ошибкой после ретраев.
#[derive(Debug)]
pub struct DownloadFailure {
    pub path: String,
    pub error: Error,
}

/// Итог `download_all`: агрегированные счётчики плюс список конкретных ошибок,
/// чтобы они не терялись за атомарными счётчиками `DownloadStats`.
#[derive(Debug)]
pub struct DownloadResult {
    pub stats: Arc<DownloadStats>,
    pub failures: Vec<DownloadFailure>,
}
