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
    api::{api::ApiClient, http_client::HttpClient, retry::RetryPolicy},
    checksum::VerifyMode,
    download::{
        job::DownloadJob,
        stats::{DownloadFailure, DownloadStats},
        worker::DownloadWorker,
    },
};

/// Пул `DownloadWorker`, получающих задачи через общий `async_channel`.
pub(crate) struct DownloadPool {
    sender: async_channel::Sender<DownloadJob>,
    handles: Vec<JoinHandle<()>>,
    failures: Arc<Mutex<Vec<DownloadFailure>>>,
}

impl DownloadPool {
    pub(crate) fn spawn(
        worker_count: usize,
        http: HttpClient,
        api: ApiClient,
        retry: RetryPolicy,
        max_link_attempts: usize,
        verify_mode: VerifyMode,
        stats: Arc<DownloadStats>,
        shutdown: CancellationToken,
    ) -> Self {
        let (sender, receiver) = bounded::<DownloadJob>(worker_count.max(1) * 100);
        let created_dirs: Arc<DashSet<PathBuf>> = Arc::new(DashSet::new());
        let failures = Arc::new(Mutex::new(Vec::new()));

        let handles = (0..worker_count.max(1))
            .map(|id| {
                let worker = DownloadWorker::new(
                    id,
                    http.clone(),
                    api.clone(),
                    retry.clone(),
                    max_link_attempts,
                    created_dirs.clone(),
                    verify_mode,
                );

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

    /// Закрывает канал (новых задач не будет) и дожидается, пока воркеры его вычерпают.
    pub(crate) async fn join(self) -> Vec<DownloadFailure> {
        drop(self.sender);

        for handle in self.handles {
            let _ = handle.await;
        }

        take(&mut *self.failures.lock().unwrap())
    }
}
