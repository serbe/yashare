use std::{
    mem::take,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_channel::bounded;
use dashmap::DashSet;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    Error,
    download::{
        DownloadContext,
        job::DownloadJob,
        stats::{DownloadFailure, DownloadStats},
        worker::DownloadWorker,
    },
};

pub(crate) struct DownloadPool {
    sender: async_channel::Sender<DownloadJob>,
    handles: Vec<JoinHandle<()>>,
    failures: Arc<Mutex<Vec<DownloadFailure>>>,
}

impl DownloadPool {
    pub(crate) fn spawn(
        worker_count: usize,
        ctx: DownloadContext,
        stats: Arc<DownloadStats>,
        shutdown: CancellationToken,
    ) -> Self {
        let (sender, receiver) = bounded::<DownloadJob>(worker_count.max(1) * 100);
        let created_dirs: Arc<DashSet<PathBuf>> = Arc::new(DashSet::new());
        let failures = Arc::new(Mutex::new(Vec::new()));

        let handles = (0..worker_count.max(1))
            .map(|id| {
                let worker = DownloadWorker::new(id, ctx.clone(), created_dirs.clone());

                tokio::spawn(worker.run(
                    receiver.clone(),
                    stats.clone(),
                    failures.clone(),
                    shutdown.clone(),
                ))
            })
            .collect();

        Self {
            sender,
            handles,
            failures,
        }
    }

    pub(crate) async fn submit(&self, job: DownloadJob) -> Result<(), Error> {
        self.sender.send(job).await.map_err(|_| Error::Cancelled)
    }

    pub(crate) async fn join(self) -> Vec<DownloadFailure> {
        drop(self.sender);

        for handle in self.handles {
            let _ = handle.await;
        }

        take(&mut *self.failures.lock().unwrap())
    }
}
