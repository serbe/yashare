use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use dashmap::DashSet;
use futures_util::StreamExt;
use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, HeaderMap, HeaderValue, RANGE},
};
use tokio::{
    fs::{OpenOptions, create_dir_all, metadata, remove_file, rename},
    io::AsyncWriteExt,
};
use tokio_util::sync::CancellationToken;

use crate::{
    CHUNK_SIZE, Error,
    api::retry::RetryDecision,
    checksum::{ChecksumSpec, VerificationMode},
    download::{
        DownloadContext,
        job::DownloadJob,
        resume::{content_range_starts_at, to_part_path},
        stats::{DownloadFailure, DownloadStats},
        verification::FileVerifier,
    },
    io_error,
    utils::sleep_or_cancel,
};

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
        }
    }

    pub(crate) fn single(ctx: DownloadContext) -> Self {
        Self::new(0, ctx, Arc::new(DashSet::new()))
    }

    pub(crate) async fn run(
        mut self,
        receiver: async_channel::Receiver<DownloadJob>,
        stats: Arc<DownloadStats>,
        failures: Arc<Mutex<Vec<DownloadFailure>>>,
        shutdown: CancellationToken,
    ) {
        loop {
            let job = tokio::select! {
                biased;

                _ = shutdown.cancelled() => break,
                job = receiver.recv() => job,
            };

            let Ok(job) = job else {
                break;
            };

            let size = job.size;
            let path = job.item_path.clone();

            match self.download_job(job, &shutdown).await {
                Ok(outcome) => stats.record(outcome, size),

                Err(Error::Cancelled) => {
                    stats.record_failure();
                    failures.lock().unwrap().push(DownloadFailure {
                        path,
                        error: Error::Cancelled,
                    });
                    break;
                }

                Err(err) => {
                    stats.record_failure();
                    tracing::error!(worker = self.id, path, error = %err, "download failed");
                    failures
                        .lock()
                        .unwrap()
                        .push(DownloadFailure { path, error: err });
                }
            }
        }
    }

    pub(crate) async fn download_job(
        &mut self,
        job: DownloadJob,
        shutdown: &CancellationToken,
    ) -> Result<Outcome, Error> {
        let mut last_error = None;

        for link_attempt in 1..=self.ctx.max_link_attempts {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

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
                        .get_download_link(&job.public_key, Some(&job.item_path), shutdown)
                        .await?
                        .href
                }
            };

            match self
                .download_to(&link, &job.destination, job.size, &job.checksum, shutdown)
                .await
            {
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
                }
                Err(err) => return Err(err),
            }
        }

        Err(Error::LinkExpired {
            path: job.item_path.clone(),
        })
        .inspect_err(|_| {
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
        shutdown: &CancellationToken,
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

        let part_path = to_part_path(destination);
        let mut last_error: Option<Error> = None;

        for attempt in 1..=self.ctx.retry.max_attempts {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            match self
                .try_download_once(
                    url,
                    &part_path,
                    destination,
                    expected_size,
                    checksum,
                    shutdown,
                )
                .await
            {
                Ok(outcome) => return Ok(outcome),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(err @ Error::Io { .. }) => return Err(err),
                Err(err) => match self.ctx.retry.decide(&err, attempt) {
                    RetryDecision::Abort => return Err(err),
                    RetryDecision::RetryAfter(delay) => {
                        last_error = Some(err);

                        if attempt >= self.ctx.retry.max_attempts {
                            break;
                        }
                        if sleep_or_cancel(delay, shutdown).await {
                            return Err(Error::Cancelled);
                        }
                    }
                },
            }
        }

        Err(last_error.unwrap_or(Error::Cancelled))
    }

    async fn try_download_once(
        &mut self,
        url: &str,
        part_path: &Path,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        shutdown: &CancellationToken,
    ) -> Result<Outcome, Error> {
        let mut existing_size = metadata(part_path).await.map(|m| m.len()).unwrap_or(0);

        if existing_size > expected_size {
            remove_file(part_path)
                .await
                .map_err(|e| io_error(part_path, e))?;
            existing_size = 0;
        }

        let mut headers = HeaderMap::new();

        if existing_size > 0 {
            let header = format!("bytes={existing_size}-");
            headers.insert(
                RANGE,
                HeaderValue::from_str(&header).map_err(|_| Error::InvalidHeader(header))?,
            );
        }

        let response = match self
            .ctx
            .http
            .send_checked(|| self.ctx.http.get(url).headers(headers.clone()))
            .await
        {
            Ok(response) => response,
            Err(err) if existing_size > 0 && err.is_range_not_satisfiable() => {
                tracing::warn!(
                    worker = self.id,
                    path = %part_path.display(),
                    "range not satisfiable, discarding partial file and restarting",
                );

                remove_file(part_path)
                    .await
                    .map_err(|e| io_error(part_path, e))?;

                return Err(Error::RangeNotSatisfiable {
                    path: part_path.to_path_buf(),
                });
            }
            Err(err) => return Err(err),
        };

        let status = response.status();
        let mut append = existing_size > 0 && status == StatusCode::PARTIAL_CONTENT;

        if existing_size > 0 && status == StatusCode::OK {
            remove_file(part_path)
                .await
                .map_err(|e| io_error(part_path, e))?;
            existing_size = 0;
            append = false;
        }

        if append {
            let valid = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
                .map(|v| content_range_starts_at(v, existing_size, expected_size))
                .unwrap_or(false);

            if !valid {
                let raw = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("<missing>")
                    .to_string();

                return Err(Error::InvalidContentRange {
                    path: part_path.into(),
                    raw,
                });
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(append)
            .write(true)
            .truncate(!append)
            .open(part_path)
            .await
            .map_err(|e| io_error(part_path, e))?;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if shutdown.is_cancelled() {
                file.flush().await.ok();
                return Err(Error::Cancelled);
            }

            let bytes = chunk.map_err(Error::StreamInterrupted)?;
            file.write_all(&bytes)
                .await
                .map_err(|e| io_error(part_path, e))?;
        }

        file.flush().await.map_err(|e| io_error(part_path, e))?;
        drop(file);

        let actual_size = metadata(part_path).await.map(|m| m.len()).unwrap_or(0);
        if actual_size != expected_size {
            return Err(Error::SizeMismatch {
                path: part_path.into(),
                expected: expected_size,
                actual: actual_size,
            });
        }

        if self.ctx.verify_mode == VerificationMode::Checksum
            && !self
                .verifier
                .verify(part_path, checksum)
                .await
                .map_err(|e| io_error(part_path, e))?
        {
            remove_file(part_path)
                .await
                .map_err(|e| io_error(part_path, e))?;
            return Err(Error::ChecksumMismatch {
                path: part_path.into(),
            });
        }

        rename(part_path, destination)
            .await
            .map_err(|e| io_error(destination, e))?;

        Ok(if existing_size > 0 {
            Outcome::Resumed
        } else {
            Outcome::Downloaded
        })
    }

    async fn ensure_dir(&self, parent: &Path) -> Result<(), Error> {
        if self.created_dirs.insert(parent.to_path_buf()) {
            create_dir_all(parent)
                .await
                .map_err(|e| io_error(parent, e))?;
        }
        Ok(())
    }
}
