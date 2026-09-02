use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_channel::Receiver;
use dashmap::DashSet;
use tokio::fs::create_dir_all;
use tracing::{debug, error, warn};

use crate::{
    Error,
    cancel::Cancel,
    download::{
        DownloadContext,
        model::{DownloadFailure, DownloadJob, DownloadStats, Outcome},
        progress::ProgressEvent,
        transport::{DownloadLinkProvider, SessionFactory},
    },
    fs::ChecksumSpec,
    io_error,
    retry::{Attempt, run},
};

/// A single download attempt for a file.
///
/// On error, emits a `Failed { retrying: true }` progress event immediately —
/// at this level we can't yet tell whether the retry policy or the caller
/// will actually retry, but from the observer's point of view "this attempt
/// failed but we're not done yet" is exactly what happened. If every attempt
/// is exhausted, `DownloadWorker::download_job` emits one final
/// `Failed { retrying: false }` on top.
struct DownloadAttempt<'a> {
    worker: &'a mut DownloadWorker,
    job_id: u64,
    item_path: &'a Arc<str>,
    url: &'a str,
    destination: &'a Path,
    expected_size: u64,
    checksum: &'a ChecksumSpec,
    cancel: &'a Cancel,
}

impl<'a> Attempt for DownloadAttempt<'a> {
    type Output = Outcome;

    async fn attempt(&mut self, _attempt_no: usize) -> Result<Outcome, Error> {
        let result = self
            .worker
            .try_download_once(
                self.job_id,
                self.item_path,
                self.url,
                self.destination,
                self.expected_size,
                self.checksum,
                self.cancel,
            )
            .await;

        if let Err(err) = &result {
            self.worker.ctx.progress.emit(ProgressEvent::Failed {
                job_id: self.job_id,
                path: self.item_path.clone(),
                error: err.to_string(),
                retrying: true,
            });
        }

        result
    }
}

/// Download worker responsible for downloading a single file.
pub(crate) struct DownloadWorker {
    id: usize,
    ctx: DownloadContext,
    created_dirs: Arc<DashSet<PathBuf>>,
    sessions: SessionFactory,
    links: DownloadLinkProvider,
}

impl DownloadWorker {
    /// Creates a new download worker with the given ID, context, and created directories.
    pub(crate) fn new(
        id: usize,
        ctx: DownloadContext,
        created_dirs: Arc<DashSet<PathBuf>>,
    ) -> Self {
        let links = DownloadLinkProvider::new(ctx.api.clone(), ctx.max_link_attempts);
        Self {
            id,
            sessions: SessionFactory::new(&ctx),
            ctx,
            created_dirs,
            links,
        }
    }

    /// Creates a new download worker with the given ID, context, and created directories.
    pub(crate) fn single(ctx: DownloadContext) -> Self {
        Self::new(0, ctx, Arc::new(DashSet::new()))
    }

    /// Runs the download worker, processing download jobs from the receiver and recording
    /// statistics and failures.
    pub(crate) async fn run(
        mut self,
        receiver: Receiver<DownloadJob>,
        stats: Arc<DownloadStats>,
        failures: Arc<Mutex<Vec<DownloadFailure>>>,
        cancel: Cancel,
    ) {
        loop {
            let job = match cancel.race(receiver.recv()).await {
                Ok(Ok(job)) => job,
                Ok(Err(_)) | Err(Error::Cancelled) => break,
                Err(_) => unreachable!(),
            };

            let size = job.size;
            let path = job.item_path.clone();

            match self.download_job(job, &cancel).await {
                Ok(outcome) => {
                    self.ctx.progress.note_bytes_done(size);
                    stats.record(outcome, size);
                },

                Err(Error::Cancelled) => {
                    stats.record_failure();
                    failures.lock().unwrap().push(DownloadFailure {
                        path: path.to_string(),
                        error: Error::Cancelled,
                    });
                    break;
                },

                Err(err) => {
                    stats.record_failure();
                    error!(worker = self.id, path = %path, error = %err, "download failed");
                    failures
                        .lock()
                        .unwrap()
                        .push(DownloadFailure { path: path.to_string(), error: err });
                },
            }
        }
    }

    /// Downloads a single job, retrying as needed until success or cancellation.
    pub(crate) async fn download_job(
        &mut self,
        job: DownloadJob,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        let mut last_error = None;

        for link_attempt in 1..=self.links.max_attempts() {
            let link = self.links.get_link(&job, link_attempt, cancel).await?;

            match self
                .download_to(
                    job.job_id,
                    &job.item_path,
                    &link,
                    &job.destination,
                    job.size,
                    &job.checksum,
                    cancel,
                )
                .await
            {
                Ok(outcome) => return Ok(outcome),
                Err(err) if err.is_expired_link() && link_attempt < self.links.max_attempts() => {
                    warn!(
                        worker = self.id,
                        path = %job.item_path,
                        attempt = link_attempt,
                        "download link expired, requesting a fresh one",
                    );
                    last_error = Some(err);
                    continue;
                },
                Err(err) => {
                    self.emit_final_failure(&job, &err);
                    return Err(err);
                },
            }
        }

        let err = Error::LinkExpired { path: job.item_path.to_string() };
        self.emit_final_failure(&job, &err);

        if let Some(prev) = last_error {
            debug!("last transient error before giving up: {prev}");
        }
        Err(err)
    }

    /// Emits the single, final `Failed { retrying: false }` event for a job
    /// that has exhausted every retry avenue (link attempts and the retry
    /// policy) and will not be attempted again.
    fn emit_final_failure(&self, job: &DownloadJob, err: &Error) {
        self.ctx.progress.emit(ProgressEvent::Failed {
            job_id: job.job_id,
            path: job.item_path.clone(),
            error: err.to_string(),
            retrying: false,
        });
    }

    /// Downloads a single file to the specified destination, retrying as needed until success or
    /// cancellation.
    #[allow(clippy::too_many_arguments)]
    async fn download_to(
        &mut self,
        job_id: u64,
        item_path: &Arc<str>,
        url: &str,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        if let Some(parent) = destination.parent() {
            self.ensure_dir(parent).await?;
        }

        if self
            .sessions
            .file_matches(destination, expected_size, checksum)
            .await
            .map_err(|e| io_error(destination, e))?
        {
            return Ok(Outcome::AlreadyComplete);
        }

        let policy = self.ctx.retry.clone();

        run(
            &policy,
            cancel,
            DownloadAttempt {
                worker: self,
                job_id,
                item_path,
                url,
                destination,
                expected_size,
                checksum,
                cancel,
            },
        )
        .await
    }

    /// Tries to download a single file once, without retrying.
    #[allow(clippy::too_many_arguments)]
    async fn try_download_once(
        &mut self,
        job_id: u64,
        item_path: &Arc<str>,
        url: &str,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        self.sessions
            .session()
            .run(job_id, item_path, url, destination, expected_size, checksum, cancel)
            .await
    }

    /// Ensures that the parent directory of the given path exists, creating it if necessary.
    async fn ensure_dir(&self, parent: &Path) -> Result<(), Error> {
        if self.created_dirs.contains(parent) {
            return Ok(());
        }

        create_dir_all(parent).await.map_err(|e| io_error(parent, e))?;

        self.created_dirs.insert(parent.to_path_buf());
        Ok(())
    }
}
