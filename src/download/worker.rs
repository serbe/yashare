use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_channel::Receiver;
use dashmap::DashSet;
use tokio::fs::create_dir_all;
use tracing::{debug, error, warn};

use crate::{
    CHUNK_SIZE, Error,
    cancel::Cancel,
    download::{
        DownloadContext, DownloadLinkProvider, DownloadSession,
        job::DownloadJob,
        resume::ResumeManager,
        stats::{DownloadFailure, DownloadStats},
    },
    fs::{ChecksumSpec, FileVerifier},
    io_error,
    retry::{Attempt, run},
};

/// A single download attempt for a file.
struct DownloadAttempt<'a> {
    worker: &'a mut DownloadWorker,
    url: &'a str,
    destination: &'a Path,
    expected_size: u64,
    checksum: &'a ChecksumSpec,
    cancel: &'a Cancel,
}

impl<'a> Attempt for DownloadAttempt<'a> {
    type Output = Outcome;

    async fn attempt(&mut self, _attempt_no: usize) -> Result<Outcome, Error> {
        self.worker
            .try_download_once(
                self.url,
                self.destination,
                self.expected_size,
                self.checksum,
                self.cancel,
            )
            .await
    }
}

/// Single file download outcome returned by download_item().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    AlreadyComplete,
    Resumed,
    Downloaded,
}

/// Download worker responsible for downloading a single file.
pub(crate) struct DownloadWorker {
    id: usize,
    ctx: DownloadContext,
    created_dirs: Arc<DashSet<PathBuf>>,
    verifier: FileVerifier,
    resume: ResumeManager,
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
            ctx,
            created_dirs,
            verifier: FileVerifier::new(CHUNK_SIZE),
            resume: ResumeManager::new(),
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
                Ok(outcome) => stats.record(outcome, size),

                Err(Error::Cancelled) => {
                    stats.record_failure();
                    failures
                        .lock()
                        .unwrap()
                        .push(DownloadFailure { path, error: Error::Cancelled });
                    break;
                },

                Err(err) => {
                    stats.record_failure();
                    error!(worker = self.id, path, error = %err, "download failed");
                    failures.lock().unwrap().push(DownloadFailure { path, error: err });
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

            match self.download_to(&link, &job.destination, job.size, &job.checksum, cancel).await {
                Ok(outcome) => return Ok(outcome),
                Err(err) if err.is_expired_link() => {
                    warn!(
                        worker = self.id,
                        path = %job.item_path,
                        attempt = link_attempt,
                        "download link expired, requesting a fresh one",
                    );
                    last_error = Some(err);
                    continue;
                },
                Err(err) => return Err(err),
            }
        }

        Err(Error::LinkExpired { path: job.item_path.clone() }).inspect_err(|_| {
            if let Some(prev) = last_error {
                debug!("last transient error before giving up: {prev}");
            }
        })
    }

    /// Downloads a single file to the specified destination, retrying as needed until success or
    /// cancellation.
    async fn download_to(
        &mut self,
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
            .verifier
            .file_matches(destination, expected_size, checksum, self.ctx.verify_mode)
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
    async fn try_download_once(
        &mut self,
        url: &str,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        let mut session = DownloadSession::new(
            &self.ctx.http,
            &self.resume,
            &mut self.verifier,
            self.ctx.verify_mode,
        );

        session.run(url, destination, expected_size, checksum, cancel).await
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
