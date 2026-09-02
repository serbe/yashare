use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use tokio::{spawn, task::JoinHandle, time::interval};

use crate::download::progress::{AggregateSnapshot, ProgressEmitter, ProgressEvent};

/// Live counters backing periodic [`AggregateSnapshot`] emission.
///
/// Kept separate from [`DownloadStats`](crate::download::model::DownloadStats)
/// deliberately: `DownloadStats` is the public, authoritative result exposed
/// via [`DownloadResult`](crate::download::model::DownloadResult), returned
/// once at the end. `ProgressAggregator` is a live, best-effort mirror used
/// only to feed periodic snapshots to an optional observer — it does not
/// replace or feed into `DownloadStats`.
#[derive(Default)]
pub(crate) struct ProgressAggregator {
    downloaded: AtomicU64,
    resumed: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
    in_flight: AtomicU64,
    total_files: AtomicU64,
    total_bytes: AtomicU64,
    bytes_done: AtomicU64,
}

impl ProgressAggregator {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn snapshot(&self) -> AggregateSnapshot {
        AggregateSnapshot {
            downloaded: self.downloaded.load(Ordering::Relaxed),
            resumed: self.resumed.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            in_flight: self.in_flight.load(Ordering::Relaxed),
            total_files: self.total_files.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
        }
    }

    /// Feeds a just-emitted event into the live counters. Called from
    /// [`ProgressEmitter::emit`] right before the event goes out on the
    /// channel, so the aggregator and the observer always see events in the
    /// same order.
    pub(crate) fn observe(&self, event: &ProgressEvent) {
        match event {
            ProgressEvent::DiscoveryFinished { total_files, total_bytes } => {
                self.total_files.store(*total_files, Ordering::Relaxed);
                self.total_bytes.store(*total_bytes, Ordering::Relaxed);
            },
            ProgressEvent::Started { .. } => {
                self.in_flight.fetch_add(1, Ordering::Relaxed);
            },
            ProgressEvent::Progress { bytes_written, .. } => {
                // bytes_written is cumulative per job, so we can't just add
                // deltas without per-job state, which defeats the point of
                // a lightweight aggregator. We approximate `bytes_done` as
                // "bytes done for completed jobs" and let the TUI show
                // per-file progress via the Progress events themselves if it
                // wants finer granularity than the aggregate provides.
                let _ = bytes_written;
            },
            ProgressEvent::Finished { outcome, .. } => {
                self.in_flight.fetch_sub(1, Ordering::Relaxed);
                match outcome {
                    crate::download::model::Outcome::Downloaded => {
                        self.downloaded.fetch_add(1, Ordering::Relaxed);
                    },
                    crate::download::model::Outcome::Resumed => {
                        self.resumed.fetch_add(1, Ordering::Relaxed);
                    },
                    crate::download::model::Outcome::AlreadyComplete => {
                        self.skipped.fetch_add(1, Ordering::Relaxed);
                    },
                }
                // Best-effort: we don't track per-job sizes here (that would
                // mean keeping O(files) state, which defeats the purpose of
                // a lightweight aggregator), so `bytes_done` only advances on
                // completion via `note_job_size`, called by the caller that
                // already knows `expected_size` at `Finished` time.
            },
            ProgressEvent::Failed { retrying: false, .. } => {
                self.in_flight.fetch_sub(1, Ordering::Relaxed);
                self.failed.fetch_add(1, Ordering::Relaxed);
            },
            _ => {},
        }
    }

    /// Adds `bytes` to the completed-bytes counter. Called alongside a
    /// `Finished` event by code that already has `expected_size` in scope
    /// (see `DownloadSession::run`), since the event itself intentionally
    /// doesn't carry size to keep `observe` a pure function of the event.
    pub(crate) fn note_bytes_done(&self, bytes: u64) {
        self.bytes_done.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// Spawns a background task that periodically emits
/// `ProgressEvent::Aggregate(..)` snapshots on the given emitter until the
/// emitter has no more receivers (or the handle is dropped/aborted).
///
/// Returns `None` if no observer is configured, since there'd be nowhere to
/// send snapshots.
pub(crate) fn spawn_aggregate_ticker(
    aggregator: Arc<ProgressAggregator>,
    emitter: ProgressEmitter,
    period: std::time::Duration,
) -> Option<JoinHandle<()>> {
    if !emitter.is_active() {
        return None;
    }

    Some(spawn(async move {
        let mut ticker = interval(period);
        loop {
            ticker.tick().await;
            if !emitter.is_active() {
                break;
            }
            emitter.emit_raw(ProgressEvent::Aggregate(aggregator.snapshot()));
        }
    }))
}
