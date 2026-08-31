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
        job::DownloadJob,
        stats::{DownloadFailure, DownloadStats},
        worker::DownloadWorker,
    },
};

/// Manages a pool of download workers, handling job submission and failure tracking.
pub(crate) struct DownloadPool {
    sender: Sender<DownloadJob>,
    handles: Vec<JoinHandle<()>>,
    failures: Arc<Mutex<Vec<DownloadFailure>>>,
}

impl DownloadPool {
    /// Spawns a new download pool with the specified number of workers and context.
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
    pub(crate) async fn submit(&self, job: DownloadJob) -> Result<(), Error> {
        self.sender.send(job).await.map_err(|_| Error::Cancelled)
    }

    /// Waits for all download workers to complete and returns any failures that occurred.
    pub(crate) async fn join(self) -> Vec<DownloadFailure> {
        drop(self.sender);

        for handle in self.handles {
            let _ = handle.await;
        }

        take(&mut *self.failures.lock().unwrap())
    }
}
