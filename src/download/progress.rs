use std::sync::Arc;

use crate::download::model::Outcome;

/// Events emitted during directory discovery and file downloads.
///
/// Progress events are sent over a channel supplied via
/// [`YaShareClientBuilder::progress_sender`](crate::client::YaShareClientBuilder::progress_sender).
/// Consumers can render these events as progress bars, logs, JSON streams,
/// or any other format — the library makes no assumptions about presentation.
///
/// # Event families
/// Events fall into two families:
///
/// ## Discovery events
/// Emitted while walking the remote directory tree, before any downloads
/// start. These are useful for showing liveness during what can be a long
/// silent pause on large folders.
///
/// ## Download events
/// Emitted per download job. `job_id` identifies a single file download and
/// is stable across all events for that job, even across retries. Consumers
/// can use `job_id` to correlate events without matching on path strings.
///
/// # Retries
/// A job may emit multiple `Failed { retrying: true }` events during
/// retries, followed by either a `Finished` event on success or one final
/// `Failed { retrying: false }` event on permanent failure.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    // ---------------------------------------------------------------
    // Discovery (tree walk) events
    // ---------------------------------------------------------------
    /// The directory walk has started.
    ///
    /// At this point, the number of files is unknown. The walker will emit
    /// `DiscoveryProgress` events as it discovers entries.
    DiscoveryStarted,

    /// The directory walk has listed another batch of entries.
    ///
    /// `scanned` is the cumulative number of items (files + directories)
    /// observed so far. There is no known total up front — the whole point
    /// of this event is to show liveness while the tree size is still
    /// unknown.
    ///
    /// # Frequency
    /// This event is emitted periodically as the walker processes pages of
    /// directory listings. The frequency depends on the page size and the
    /// number of items in the directory.
    DiscoveryProgress { scanned: u64 },

    /// The directory walk has finished.
    ///
    /// This event is emitted once, after which only download events follow.
    /// It provides the final counts of files and total bytes.
    DiscoveryFinished { total_files: u64, total_bytes: u64 },

    // ---------------------------------------------------------------
    // Per-file download events
    // ---------------------------------------------------------------
    /// A download job has started (or restarted from scratch).
    ///
    /// This is emitted at the beginning of a download, or when a corrupt
    /// partial file is discarded and the download restarts from zero.
    ///
    /// # `job_id` correlation
    /// Consumers should store `job_id -> path` mapping on this event and use
    /// it for subsequent events to avoid storing `path` in every event.
    Started {
        job_id: u64,
        path: Arc<str>,
        total_size: u64,
    },

    /// A chunk of bytes has been written for this job.
    ///
    /// This event fires once per received chunk (typically every few KiB to
    /// a few MiB). To avoid a clone/alloc per chunk, it deliberately does
    /// not carry the `path` — consumers that need it should have a
    /// `job_id -> path` mapping from the `Started` event.
    ///
    /// # Cumulative values
    /// `bytes_written` is the **cumulative** number of bytes written to the
    /// partial file so far, including any bytes already present from a
    /// resumed download. Consumers can set a progress bar's position
    /// directly from this value without accumulating deltas.
    Progress {
        job_id: u64,
        bytes_written: u64,
        total_size: u64,
    },

    /// A download job finished successfully.
    ///
    /// The `outcome` indicates whether the file was downloaded from scratch,
    /// resumed, or was already complete and skipped.
    Finished {
        job_id: u64,
        path: Arc<str>,
        outcome: Outcome,
    },

    /// A download job attempt failed.
    ///
    /// # Retry semantics
    /// - `retrying: true` is emitted after *every* failed attempt, including the last one — at the
    ///   point an individual attempt fails, the library cannot yet tell whether retry logic will
    ///   try again.
    /// - If that was in fact the last attempt, exactly one more `Failed { retrying: false }`
    ///   follows for the same `job_id`, marking the job as permanently abandoned.
    ///
    /// # UI usage
    /// - A UI that only cares about final outcomes should filter for `retrying: false`.
    /// - A UI that wants to show live "retrying..." status can use the `retrying: true` events
    ///   directly.
    Failed {
        job_id: u64,
        path: Arc<str>,
        error: String,
        retrying: bool,
    },

    // ---------------------------------------------------------------
    // Aggregate snapshot
    // ---------------------------------------------------------------
    /// A point-in-time snapshot of overall progress across all jobs.
    ///
    /// Emitted periodically (see
    /// [`YaShareClientBuilder::aggregate_interval`](crate::client::YaShareClientBuilder::aggregate_interval))
    /// in addition to per-job events. A UI that only wants a single overall
    /// progress bar can listen for just this variant and ignore everything
    /// else — no need to track per-file state at all.
    Aggregate(AggregateSnapshot),
}

impl ProgressEvent {
    /// Returns the `job_id` associated with this event, if any.
    ///
    /// `Discovery*` and `Aggregate` events are not tied to a single job and
    /// return `None`.
    pub fn job_id(&self) -> Option<u64> {
        match self {
            Self::Started { job_id, .. }
            | Self::Progress { job_id, .. }
            | Self::Finished { job_id, .. }
            | Self::Failed { job_id, .. } => Some(*job_id),
            Self::DiscoveryStarted
            | Self::DiscoveryProgress { .. }
            | Self::DiscoveryFinished { .. }
            | Self::Aggregate(_) => None,
        }
    }
}

/// A point-in-time snapshot of aggregate download progress.
///
/// Mirrors [`DownloadStats`](crate::download::model::DownloadStats) but as a
/// plain owned struct suitable for sending over a channel repeatedly.
#[derive(Debug, Clone, Copy, Default)]
pub struct AggregateSnapshot {
    /// Files downloaded from scratch so far.
    pub downloaded: u64,
    /// Files resumed from a partial download so far.
    pub resumed: u64,
    /// Files already complete and skipped so far.
    pub skipped: u64,
    /// Files that failed permanently so far.
    pub failed: u64,
    /// Files currently being downloaded (started but not yet finished/failed).
    pub in_flight: u64,
    /// Total files known so far (from discovery; may still grow while walking).
    pub total_files: u64,
    /// Total bytes across all known files (from discovery; may still grow).
    pub total_bytes: u64,
    /// Bytes written so far across all jobs, completed or in-flight.
    pub bytes_done: u64,
}

/// Internal handle for emitting progress events.
///
/// Wraps an optional channel sender and an optional aggregator. The hot path
/// (no observer configured) is a single `is_some` check with no allocation
/// and no channel overhead.
///
/// The aggregator is fed before the observer sees the event, ensuring that
/// aggregate snapshots stay consistent with the exact sequence of events an
/// observer sees.
#[derive(Clone, Default)]
pub(crate) struct ProgressEmitter {
    sender: Option<async_channel::Sender<ProgressEvent>>,
    aggregator: Option<Arc<crate::download::aggregator::ProgressAggregator>>,
}

impl ProgressEmitter {
    /// Creates a new emitter with an optional sender and aggregator.
    pub(crate) fn new(
        sender: Option<async_channel::Sender<ProgressEvent>>,
        aggregator: Option<Arc<crate::download::aggregator::ProgressAggregator>>,
    ) -> Self {
        Self { sender, aggregator }
    }

    /// Returns a disabled emitter that discards all events.
    pub(crate) fn none() -> Self {
        Self { sender: None, aggregator: None }
    }

    /// Replaces the aggregator on this emitter.
    ///
    /// Used by `download_all` to scope a fresh aggregator to a single call,
    /// ensuring concurrent `download_all` calls on a cloned client don't
    /// share aggregate counters.
    pub(crate) fn with_aggregator(
        &self,
        aggregator: Arc<crate::download::aggregator::ProgressAggregator>,
    ) -> Self {
        Self {
            sender: self.sender.clone(),
            aggregator: Some(aggregator),
        }
    }

    /// Emits an event: feeds the aggregator (if any) and forwards to the
    /// observer channel (if any).
    ///
    /// # Delivery semantics
    /// Events are sent with `try_send` — if the receiver is full or closed,
    /// the event is silently dropped. Progress reporting must never cause a
    /// download to fail or block.
    #[inline]
    pub(crate) fn emit(&self, event: ProgressEvent) {
        if let Some(aggregator) = &self.aggregator {
            aggregator.observe(&event);
        }
        self.emit_raw(event);
    }

    /// Sends an event directly to the observer channel without touching the
    /// aggregator.
    ///
    /// Used by the aggregate ticker to avoid feeding its own `Aggregate`
    /// events back into the counters.
    #[inline]
    pub(crate) fn emit_raw(&self, event: ProgressEvent) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(event);
        }
    }

    /// Returns `true` if there is an observer configured.
    #[inline]
    pub(crate) fn is_active(&self) -> bool {
        self.sender.is_some()
    }

    /// Adds `bytes` to the completed-bytes counter of the attached
    /// aggregator, if any.
    ///
    /// This is a no-op if there is no aggregator (e.g., when this emitter
    /// belongs to a `download_item` call, which doesn't use one).
    #[inline]
    pub(crate) fn note_bytes_done(&self, bytes: u64) {
        if let Some(aggregator) = &self.aggregator {
            aggregator.note_bytes_done(bytes);
        }
    }
}
