use std::path::{Path, PathBuf};

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
    Error,
    client::{HttpClient, RetryDecision, RetryPolicy},
    io_error,
    utils::sleep_or_cancel,
    verify::Verifier,
};

#[derive(Clone)]
pub struct DownloadClient {
    http: HttpClient,
    retry: RetryPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    AlreadyComplete,
    Resumed,
    Downloaded,
}

impl DownloadClient {
    pub fn new(http: HttpClient, retry: RetryPolicy) -> Self {
        Self { http, retry }
    }

    pub async fn download(
        &self,
        url: &str,
        destination: &Path,
        expected_size: u64,
        verify: Verifier,
        shutdown: &CancellationToken,
    ) -> Result<Outcome, Error> {
        if let Some(parent) = destination.parent() {
            create_dir_all(parent)
                .await
                .map_err(|e| io_error(parent, e))?;
        }

        if file_matches(destination, expected_size, &verify)
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
                    &verify,
                    shutdown,
                )
                .await
            {
                Ok(outcome) => return Ok(outcome),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(err @ Error::Io { .. }) => return Err(err), // never retryable
                Err(err) => {
                    let decision = self.retry.decide(&err, attempt);

                    match decision {
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
                    }
                }
            }
        }

        Err(last_error.unwrap_or(Error::Cancelled))
    }

    async fn attempt(
        &self,
        url: &str,
        part_path: &Path,
        destination: &Path,
        expected_size: u64,
        verify: &Verifier,
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

        let response = self
            .http
            .execute(
                || self.http.get(url).headers(headers.clone()),
                &self.retry,
                shutdown,
            )
            .await?;

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

        let ok = verify(part_path)
            .await
            .map_err(|e| io_error(part_path, e))?;
        if !ok {
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
}

fn part_path_for(destination: &Path) -> PathBuf {
    let mut path = destination.as_os_str().to_os_string();
    path.push(".part");
    PathBuf::from(path)
}

async fn file_matches(path: &Path, expected_size: u64, verify: &Verifier) -> std::io::Result<bool> {
    let metadata = match metadata(path).await {
        Ok(m) => m,
        Err(_) => return Ok(false),
    };
    if metadata.len() != expected_size {
        return Ok(false);
    }
    verify(path).await
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
