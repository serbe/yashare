use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use bytes::BytesMut;
use dashmap::DashSet;
use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, HeaderMap, HeaderValue, RANGE},
};
use sha2::Sha256;
use tokio::{
    fs::{File, OpenOptions, create_dir_all, metadata, remove_file, rename},
    io::{AsyncReadExt, AsyncWriteExt},
};
use tokio_util::sync::CancellationToken;

use crate::{
    CHUNK_SIZE, Error,
    api::{
        api::ApiClient,
        http_client::HttpClient,
        retry::{RetryDecision, RetryPolicy},
    },
    checksum::{ChecksumSpec, VerifyMode},
    download::{
        job::DownloadJob,
        stats::{DownloadFailure, DownloadStats},
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

/// Долгоживущий воркер. Пул таких воркеров вычитывает `DownloadJob` из
/// общего `async_channel` и обрабатывает их по одному: обновляет
/// протухшие ссылки, докачивает `.part`-файлы, проверяет чексуммы.
pub(crate) struct DownloadWorker {
    id: usize,
    http: HttpClient,
    api: ApiClient,
    retry: RetryPolicy,
    max_link_attempts: usize,
    created_dirs: Arc<DashSet<PathBuf>>,
    buffer: BytesMut,
    verify_mode: VerifyMode,
}

impl DownloadWorker {
    pub(crate) fn new(
        id: usize,
        http: HttpClient,
        api: ApiClient,
        retry: RetryPolicy,
        max_link_attempts: usize,
        created_dirs: Arc<DashSet<PathBuf>>,
        verify_mode: VerifyMode,
    ) -> Self {
        let mut buffer = BytesMut::with_capacity(CHUNK_SIZE);
        buffer.resize(CHUNK_SIZE, 0);

        Self {
            id,
            http,
            api,
            retry,
            max_link_attempts,
            created_dirs,
            buffer,
            verify_mode,
        }
    }

    /// Забирает задачи из `receiver`, пока канал не закроется или не сработает `shutdown`.
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
                break; // канал закрыт продюсером — работы больше нет
            };

            let size = job.size;
            let path = job.item_path.clone();

            match self.process(job, &shutdown).await {
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

    /// Получает (при необходимости — обновлённую) ссылку и качает файл целиком.
    pub(crate) async fn process(
        &mut self,
        job: DownloadJob,
        shutdown: &CancellationToken,
    ) -> Result<Outcome, Error> {
        let mut last_error = None;

        for refresh in 1..=self.max_link_attempts {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let href = if refresh == 1 {
                job.initial_href.clone()
            } else {
                None
            };

            let link = match href {
                Some(href) => href,
                None => {
                    self.api
                        .download_href(&job.public_key, Some(&job.item_path), shutdown)
                        .await?
                        .href
                }
            };

            match self
                .download_to(&link, &job.destination, job.size, &job.checksum, shutdown)
                .await
            {
                Ok(outcome) => return Ok(outcome),
                Err(err) if is_expired_link_error(&err) => {
                    tracing::warn!(
                        worker = self.id,
                        path = %job.item_path,
                        attempt = refresh,
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
            .file_matches(destination, expected_size, checksum)
            .await
            .map_err(|e| io_error(destination, e))?
        {
            return Ok(Outcome::AlreadyComplete);
        }

        let part_path = part_path_for(destination);
        let mut last_error: Option<Error> = None;

        for attempt in 1..=self.retry.max_attempts {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            match self
                .attempt(
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
                Err(err @ Error::Io { .. }) => return Err(err), // не ретраим
                Err(err) => match self.retry.decide(&err, attempt) {
                    RetryDecision::Abort => return Err(err),
                    RetryDecision::RetryAfter(delay) => {
                        last_error = Some(err);

                        if attempt >= self.retry.max_attempts {
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

    async fn attempt(
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
            .http
            .send_once(|| self.http.get(url).headers(headers.clone()))
            .await
        {
            Ok(response) => response,

            // Сервер не может докачать с этим Range: .part битый, устаревший
            // или сервер вообще не поддерживает докачку по этому href.
            // Повторный запрос с тем же Range снова получит 416, поэтому
            // сбрасываем .part и просим внешний ретрай начать с нуля.
            Err(err) if existing_size > 0 && is_range_not_satisfiable(&err) => {
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

        if self.verify_mode == VerifyMode::Checksum
            && !self
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
        if self.created_dirs.contains(parent) {
            return Ok(());
        }
        if self.created_dirs.insert(parent.to_path_buf()) {
            create_dir_all(parent)
                .await
                .map_err(|e| io_error(parent, e))?;
        }
        Ok(())
    }

    async fn file_matches(
        &mut self,
        path: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
    ) -> std::io::Result<bool> {
        let metadata = match metadata(path).await {
            Ok(m) => m,
            Err(_) => return Ok(false),
        };
        if metadata.len() != expected_size {
            return Ok(false);
        }

        match self.verify_mode {
            VerifyMode::SizeOnly => Ok(true),
            VerifyMode::Checksum => self.verify(path, checksum).await,
        }
    }

    async fn hash_file<D: Digest + Default>(&mut self, path: &Path) -> std::io::Result<String> {
        let mut file = File::open(path).await?;
        let mut hasher = D::default();

        loop {
            let n = file.read(&mut self.buffer[..]).await?;
            if n == 0 {
                break;
            }
            hasher.update(&self.buffer[..n]);
        }

        Ok(hex::encode(hasher.finalize()))
    }

    async fn hash_file_both(&mut self, path: &Path) -> std::io::Result<(String, String)> {
        let mut file = File::open(path).await?;
        let mut md5 = Md5::default();
        let mut sha256 = Sha256::default();

        loop {
            let n = file.read(&mut self.buffer[..]).await?;
            if n == 0 {
                break;
            }
            let chunk = &self.buffer[..n];
            md5.update(chunk);
            sha256.update(chunk);
        }

        Ok((hex::encode(md5.finalize()), hex::encode(sha256.finalize())))
    }

    async fn verify(&mut self, path: &Path, checksum: &ChecksumSpec) -> std::io::Result<bool> {
        match checksum {
            ChecksumSpec::Md5(expected) => {
                let actual = self.hash_file::<Md5>(path).await?;
                Ok(actual.eq_ignore_ascii_case(expected))
            }
            ChecksumSpec::Sha256(expected) => {
                let actual = self.hash_file::<Sha256>(path).await?;
                Ok(actual.eq_ignore_ascii_case(expected))
            }
            ChecksumSpec::Both { md5, sha256 } => {
                let (actual_md5, actual_sha256) = self.hash_file_both(path).await?;
                Ok(actual_md5.eq_ignore_ascii_case(md5)
                    && actual_sha256.eq_ignore_ascii_case(sha256))
            }
            ChecksumSpec::None => Ok(true),
        }
    }
}

fn is_range_not_satisfiable(error: &Error) -> bool {
    matches!(
        error,
        Error::Status { status, .. } | Error::Api { status, .. }
            if *status == StatusCode::RANGE_NOT_SATISFIABLE
    )
}

fn is_expired_link_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Status { status, .. } | Error::Api { status, .. }
            if matches!(status, &StatusCode::FORBIDDEN | &StatusCode::GONE)
    )
}

fn part_path_for(destination: &Path) -> PathBuf {
    let mut path = destination.as_os_str().to_os_string();
    path.push(".part");
    PathBuf::from(path)
}

fn content_range_starts_at(header_value: &str, expected_start: u64, expected_total: u64) -> bool {
    let Some(rest) = header_value.strip_prefix("bytes ") else {
        return false;
    };
    let mut parts = rest.split('/');
    let (Some(range), Some(total)) = (parts.next(), parts.next()) else {
        return false;
    };
    let mut range_parts = range.split('-');
    let Some(start) = range_parts.next().and_then(|s| s.parse::<u64>().ok()) else {
        return false;
    };
    let Some(total) = total.parse::<u64>().ok() else {
        return false;
    };
    start == expected_start && total == expected_total
}
