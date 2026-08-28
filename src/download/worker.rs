use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use dashmap::DashSet;
use futures_util::StreamExt;
use reqwest::header::HeaderMap;
use tokio::{
    fs::{create_dir_all, rename},
    io::AsyncWriteExt,
};

use crate::{
    CHUNK_SIZE, Error,
    api::send_checked,
    cancel::Cancel,
    checksum::{ChecksumSpec, VerificationMode},
    download::{
        DownloadContext,
        job::DownloadJob,
        resume::{ResumeAction, ResumeManager, ResumeState},
        stats::{DownloadFailure, DownloadStats},
        verification::FileVerifier,
    },
    io_error, retry,
};

struct DownloadAttempt<'a> {
    worker: &'a mut DownloadWorker,
    url: &'a str,
    destination: &'a Path,
    expected_size: u64,
    checksum: &'a ChecksumSpec,
    cancel: &'a Cancel,
}

impl<'a> retry::Attempt for DownloadAttempt<'a> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    AlreadyComplete,
    Resumed,
    Downloaded,
}

pub(crate) struct DownloadWorker {
    id: usize,
    ctx: DownloadContext,
    created_dirs: Arc<DashSet<PathBuf>>,
    verifier: FileVerifier,
    resume: ResumeManager,
}

impl DownloadWorker {
    pub(crate) fn new(
        id: usize,
        ctx: DownloadContext,
        created_dirs: Arc<DashSet<PathBuf>>,
    ) -> Self {
        Self {
            id,
            ctx,
            created_dirs,
            verifier: FileVerifier::new(CHUNK_SIZE),
            resume: ResumeManager::new(),
        }
    }

    pub(crate) fn single(ctx: DownloadContext) -> Self {
        Self::new(0, ctx, Arc::new(DashSet::new()))
    }

    async fn finalize_existing_part(
        &mut self,
        state: &ResumeState,
        destination: &Path,
        checksum: &ChecksumSpec,
    ) -> Result<(), Error> {
        if self.ctx.verify_mode == VerificationMode::Checksum
            && !self
                .verifier
                .verify(&state.part_path, checksum)
                .await
                .map_err(|e| io_error(&state.part_path, e))?
        {
            self.resume.reset(state).await?;

            return Err(Error::ChecksumMismatch { path: state.part_path.clone() });
        }

        rename(&state.part_path, destination)
            .await
            .map_err(|e| io_error(destination, e))?;

        Ok(())
    }

    pub(crate) async fn run(
        mut self,
        receiver: async_channel::Receiver<DownloadJob>,
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
                    tracing::error!(worker = self.id, path, error = %err, "download failed");
                    failures.lock().unwrap().push(DownloadFailure { path, error: err });
                },
            }
        }
    }

    pub(crate) async fn download_job(
        &mut self,
        job: DownloadJob,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        let mut last_error = None;

        for link_attempt in 1..=self.ctx.max_link_attempts {
            cancel.check()?;

            let href = if link_attempt == 1 {
                job.initial_href.clone()
            } else {
                None
            };

            let link = match href {
                Some(href) => href,
                None => {
                    self.ctx
                        .api
                        .get_download_link(&job.public_key, Some(&job.item_path), cancel)
                        .await?
                        .href
                },
            };

            match self.download_to(&link, &job.destination, job.size, &job.checksum, cancel).await {
                Ok(outcome) => return Ok(outcome),
                Err(err) if err.is_expired_link() => {
                    tracing::warn!(
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
                tracing::debug!("last transient error before giving up: {prev}");
            }
        })
    }

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

        // Clone the policy out *before* borrowing `self` mutably below —
        // `self.ctx.retry` and `DownloadAttempt { worker: self, .. }`
        // would otherwise be a simultaneous shared + mutable borrow of `self`.
        let policy = self.ctx.retry.clone();

        retry::run(
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

    async fn try_download_once(
        &mut self,
        url: &str,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        let mut state = self.resume.inspect(destination, expected_size).await?;

        if matches!(state.action, ResumeAction::Finalize) {
            self.finalize_existing_part(&state, destination, checksum).await?;

            return Ok(Outcome::AlreadyComplete);
        }

        if matches!(state.action, ResumeAction::Restart) {
            self.resume.reset(&state).await?;

            state = self.resume.inspect(destination, expected_size).await?;
        }

        let mut headers = HeaderMap::new();

        self.resume.apply_range(&mut headers, &state)?;

        let response =
            match send_checked(&self.ctx.http, self.ctx.http.get(url).headers(headers.clone()))
                .await
            {
                Ok(response) => response,

                Err(err) if state.is_resuming() && err.is_range_not_satisfiable() => {
                    tracing::warn!(
                        worker = self.id,
                        path = %state.part_path.display(),
                        "range not satisfiable, discarding partial file"
                    );

                    self.resume.reset(&state).await?;

                    return Err(Error::RangeNotSatisfiable { path: state.part_path.clone() });
                },

                Err(err) => return Err(err),
            };

        state = self.resume.validate_response(
            &state,
            response.status(),
            response.headers(),
            expected_size,
        )?;

        if matches!(state.action, ResumeAction::Restart) {
            tracing::warn!(
                worker = self.id,
                path = %state.part_path.display(),
                "server ignored Range header, restarting download"
            );

            self.resume.reset(&state).await?;

            return Err(Error::RangeNotSatisfiable { path: state.part_path.clone() });
        }

        let mut file = self.resume.open(&state).await?;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if cancel.check().is_err() {
                let _ = file.flush().await;
                return Err(Error::Cancelled);
            }

            let bytes = chunk.map_err(Error::StreamInterrupted)?;

            file.write_all(&bytes)
                .await
                .map_err(|error| io_error(&state.part_path, error))?;
        }

        file.flush().await.map_err(|e| io_error(&state.part_path, e))?;

        drop(file);

        let actual_size = self.resume.current_size(&state).await?;

        if actual_size != expected_size {
            return Err(Error::SizeMismatch {
                path: state.part_path.into(),
                expected: expected_size,
                actual: actual_size,
            });
        }

        if self.ctx.verify_mode == VerificationMode::Checksum
            && !self
                .verifier
                .verify(&state.part_path, checksum)
                .await
                .map_err(|error| io_error(&state.part_path, error))?
        {
            self.resume.reset(&state).await?;

            return Err(Error::ChecksumMismatch { path: state.part_path.clone() });
        }

        rename(&state.part_path, destination)
            .await
            .map_err(|e| io_error(destination, e))?;

        Ok(if state.is_resuming() {
            Outcome::Resumed
        } else {
            Outcome::Downloaded
        })
    }

    async fn ensure_dir(&self, parent: &Path) -> Result<(), Error> {
        if self.created_dirs.insert(parent.to_path_buf()) {
            create_dir_all(parent).await.map_err(|e| io_error(parent, e))?;
        }
        Ok(())
    }
}
