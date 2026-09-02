use std::{
    mem::take,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_channel::{Sender, bounded};
use dashmap::DashSet;
use tokio::{spawn, task::JoinHandle};

use crate::{
    Error,
    cancel::Cancel,
    download::{
        DownloadContext,
        execution::DownloadWorker,
        model::{DownloadFailure, DownloadJob, DownloadStats},
    },
};

/// A pool of download workers that processes jobs concurrently.
///
/// `DownloadPool` manages a fixed number of worker tasks that consume
/// `DownloadJob`s from a bounded channel. The pool handles:
/// - Worker lifecycle (spawning, joining)
/// - Job distribution (workers pull from a shared queue)
/// - Failure collection (all failures are aggregated)
/// - Directory creation (workers coordinate to avoid race conditions)
///
/// # Concurrency
/// Workers run in separate Tokio tasks and pull jobs from the channel
/// concurrently. The channel is bounded to prevent unbounded memory growth
/// when workers are slower than job production.
///
/// # Shutdown
/// Dropping the `DownloadPool` closes the job channel. Workers exit when the
/// channel is empty and closed. Call `join()` to wait for all workers to
/// finish and collect any accumulated failures.
pub(crate) struct DownloadPool {
    sender: Sender<DownloadJob>,
    handles: Vec<JoinHandle<()>>,
    failures: Arc<Mutex<Vec<DownloadFailure>>>,
}

impl DownloadPool {
    /// Spawns a new download pool with the specified number of workers.
    ///
    /// # Arguments
    /// - `worker_count`: Number of concurrent downloads. At least 1.
    /// - `ctx`: Shared download context (HTTP client, API client, etc.).
    /// - `stats`: Shared statistics counter.
    /// - `cancel`: Cancellation token shared with all workers.
    ///
    /// # Channel capacity
    /// The job channel capacity is `worker_count * 100`. This provides enough
    /// buffering to keep workers busy while still bounding memory usage.
    pub(crate) fn spawn(
        worker_count: usize,
        ctx: DownloadContext,
        stats: Arc<DownloadStats>,
        cancel: Cancel,
    ) -> Self {
        let (sender, receiver) = bounded::<DownloadJob>(worker_count.max(1) * 100);
        let created_dirs: Arc<DashSet<PathBuf>> = Arc::new(DashSet::new());
        let failures = Arc::new(Mutex::new(Vec::new()));

        let handles = (0..worker_count.max(1))
            .map(|id| {
                let worker = DownloadWorker::new(id, ctx.clone(), created_dirs.clone());

                spawn(worker.run(receiver.clone(), stats.clone(), failures.clone(), cancel.clone()))
            })
            .collect();

        Self { sender, handles, failures }
    }

    /// Submits a download job to the pool.
    ///
    /// This returns immediately after enqueuing the job. The job will be
    /// picked up by the next available worker.
    ///
    /// # Errors
    /// Returns `Error::Cancelled` if the pool has been dropped (the
    /// receiver is closed), which typically indicates shutdown.
    pub(crate) async fn submit(&self, job: DownloadJob) -> Result<(), Error> {
        self.sender.send(job).await.map_err(|_| Error::Cancelled)
    }

    /// Waits for all workers to finish and returns any failures.
    ///
    /// This closes the job channel (preventing new submissions) and waits for
    /// all worker tasks to complete. The returned `Vec<DownloadFailure>`
    /// contains all failures that occurred during processing.
    ///
    /// # Note
    /// After calling `join()`, the pool is consumed and cannot be used again.
    pub(crate) async fn join(self) -> Vec<DownloadFailure> {
        drop(self.sender);

        for handle in self.handles {
            let _ = handle.await;
        }

        take(&mut *self.failures.lock().unwrap())
    }
}
