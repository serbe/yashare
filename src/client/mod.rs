mod builder;

use std::{path::Path, sync::Arc};

use futures_util::{Stream, TryStreamExt};
use reqwest::Client;
use tracing::error;

use crate::{
    Error,
    api::{HttpClient, ResourceClient},
    cancel::Cancel,
    client::builder::{ClientConfig, YaShareClientBuilder},
    download::{
        DownloadContext,
        aggregator::{ProgressAggregator, spawn_aggregate_ticker},
        concurrency::DownloadPool,
        execution::DownloadWorker,
        model::{DownloadFailure, DownloadJob, DownloadResult, DownloadStats, Outcome},
        progress::ProgressEmitter,
    },
    error::ClientError,
    model::{Item, PublicKey, Resource},
    walker::{ParallelWalker, Walker},
};

/// The main client for interacting with Yandex.Disk public shares.
///
/// `YaShareClient` provides the primary interface for downloading files and
/// folders from public Yandex.Disk shares.
///
/// # Features
/// - Download individual files or entire folders.
/// - Resume interrupted downloads.
/// - Verify downloaded files with checksums.
/// - Progress reporting via channels.
/// - Configurable concurrency and retry policies.
///
/// # Example
/// ```no_run
/// use yashare::{Cancel, PublicKey, YaShareClient};
///
/// # async fn example() -> Result<(), yashare::Error> {
/// let client = YaShareClient::default();
/// let key = PublicKey::parse("https://disk.yandex.ru/d/abc123")?;
/// let cancel = Cancel::new();
///
/// let result = client.download_all(&key, "./downloads".as_ref(), &cancel).await?;
///
/// println!("Downloaded {} files", result.stats.downloaded());
/// # Ok(())
/// # }
/// ```
///
/// # Cloning
/// `YaShareClient` implements `Clone` and can be shared across threads.
/// The internal HTTP client and API client are cheap to clone (they share
/// connection pools and configuration).
#[derive(Clone)]
pub struct YaShareClient {
    ctx: DownloadContext,
    max_concurrent_downloads: usize,
    max_listing_workers: usize,
    aggregate_interval: std::time::Duration,
}

impl Default for YaShareClient {
    /// Creates a default [`YaShareClient`] using the [`YaShareClientBuilder`]
    /// with default configuration.
    ///
    /// Default settings:
    /// - 6 concurrent downloads
    /// - 6 listing workers
    /// - 8 retry attempts with exponential backoff
    /// - Size-only verification
    /// - 3 link refresh attempts
    fn default() -> Self {
        YaShareClientBuilder::default()
            .build()
            .expect("failed to create default client")
    }
}

impl YaShareClient {
    /// Returns a new builder for configuring a [`YaShareClient`].
    ///
    /// # Example
    /// ```no_run
    /// use std::time::Duration;
    ///
    /// use yashare::YaShareClient;
    ///
    /// let client = YaShareClient::builder()
    ///     .max_concurrent_downloads(10)
    ///     .timeout(Duration::from_secs(60))
    ///     .build()?;
    /// # Ok::<(), yashare::Error>(())
    /// ```
    pub fn builder() -> YaShareClientBuilder {
        YaShareClientBuilder::new()
    }

    /// Creates a [`YaShareClient`] from an HTTP client and configuration.
    ///
    /// This is used internally by the builder. Most callers should use
    /// `YaShareClient::builder()` instead.
    pub(crate) fn from_parts(http: Client, config: ClientConfig) -> Result<Self, Error> {
        let http = HttpClient::new(http);

        let api =
            ResourceClient::new(http.clone(), config.retry_policy.clone(), config.api_base.clone());

        let progress = ProgressEmitter::new(config.progress_sender, None);

        let ctx = DownloadContext {
            http,
            api,
            retry: config.retry_policy,
            max_link_attempts: config.max_link_attempts,
            verify_mode: config.verify_mode,
            progress,
        };

        Ok(Self {
            ctx,
            max_concurrent_downloads: config.max_concurrent_downloads,
            max_listing_workers: config.max_listing_workers,
            aggregate_interval: config.aggregate_interval,
        })
    }

    /// Sets the fields to request from the API for resource metadata.
    ///
    /// This controls which fields are included in responses from the
    /// Yandex.Disk API. Reducing the field set improves performance by
    /// reducing response size.
    ///
    /// See [`model::ResourceField`] for available fields.
    pub fn set_fields(&mut self, fields: String) {
        self.ctx.api.set_fields(fields);
    }

    /// Resolves the root path of a public folder.
    ///
    /// Fetches metadata for the root of the share and verifies that it is a
    /// folder (not a file). Returns the path of the root folder.
    ///
    /// # Errors
    /// Returns `ClientError::NotAFolder` if the public key points to a file
    /// rather than a folder.
    async fn resolve_root_path(
        &self,
        public_key: &PublicKey,
        cancel: &Cancel,
    ) -> Result<String, Error> {
        let root = self.ctx.api.resource_meta(public_key, None, cancel).await?;

        if !root.is_dir() {
            return Err(Error::Client(ClientError::NotAFolder(
                root.name.unwrap_or_else(|| public_key.as_api_string()),
            )));
        }

        Ok(root.path.unwrap_or_else(|| "/".to_string()))
    }

    /// Fetches metadata for a resource (file or folder) from the API.
    ///
    /// This is a low-level method that returns the raw `Resource` response
    /// from the API. For downloading, prefer `download_item()` or
    /// `download_all()`.
    ///
    /// # Arguments
    /// - `public_key`: The public key identifying the share.
    /// - `path`: The path within the share, or `None` for the root.
    /// - `cancel`: Cancellation token.
    pub async fn resource_meta(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Resource, Error> {
        self.ctx.api.resource_meta(public_key, path, cancel).await
    }

    /// Returns a stream of all items in a public folder (recursive).
    ///
    /// This walks the entire directory tree and yields every item (files and
    /// folders) as a stream. The stream is lazy — items are fetched as the
    /// stream is consumed.
    ///
    /// # Performance
    /// For large directories, this may make many API requests. Consider
    /// using `download_all()` if you intend to download all files.
    ///
    /// # Example
    /// ```no_run
    /// # use yashare::{YaShareClient, PublicKey, Cancel};
    /// # async fn example() -> Result<(), yashare::Error> {
    /// # let client = YaShareClient::default();
    /// # let key = PublicKey::parse("...")?;
    /// # let cancel = Cancel::new();
    /// use futures_util::TryStreamExt;
    ///
    /// let mut stream = client.walk(&key, &cancel).await?;
    /// while let Some(item) = stream.try_next().await? {
    ///     println!("Found: {:?}", item.path);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn walk(
        &self,
        public_key: &PublicKey,
        cancel: &Cancel,
    ) -> Result<impl Stream<Item = Result<Item, Error>>, Error> {
        let root_path = self.resolve_root_path(public_key, cancel).await?;

        Ok(Walker::new(self.ctx.api.clone(), public_key, root_path, cancel.clone()).into_stream())
    }

    /// Downloads a single item from the API to the specified directory.
    ///
    /// This downloads a single file. If the item is a directory, returns an
    /// error (use `download_all()` for folders).
    ///
    /// # Returns
    /// - `Outcome::Downloaded`: File was downloaded from scratch.
    /// - `Outcome::Resumed`: File was resumed from a partial download.
    /// - `Outcome::AlreadyComplete`: File already existed and matched.
    ///
    /// # Example
    /// ```no_run
    /// # use yashare::{YaShareClient, PublicKey, Cancel};
    /// # async fn example() -> Result<(), yashare::Error> {
    /// # let client = YaShareClient::default();
    /// # let key = PublicKey::parse("...")?;
    /// # let cancel = Cancel::new();
    /// let item = client.resource_meta(&key, None, &cancel).await?;
    /// let outcome = client.download_item(&key, &item, "./".as_ref(), &cancel).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn download_item(
        &self,
        public_key: &PublicKey,
        item: &Item,
        dest_dir: &Path,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        let job = DownloadJob::for_download(dest_dir, public_key, item)?;

        let mut worker = DownloadWorker::single(self.ctx.clone());

        worker.download_job(job, cancel).await
    }

    /// Downloads all items in a public folder to the specified directory.
    ///
    /// This recursively walks the folder tree and downloads every file.
    /// The directory structure is preserved relative to the share root.
    ///
    /// # Progress reporting
    /// If a progress observer is configured (see
    /// [`YaShareClientBuilder::progress_sender`]), this emits:
    /// - `Discovery*` events while walking the tree.
    /// - Per-job events for each download (`Started`, `Progress`, `Finished`, `Failed`).
    /// - Periodic `Aggregate` snapshots for the duration of the call.
    ///
    /// The aggregate ticker is stopped before returning.
    ///
    /// # Returns
    /// Returns `DownloadResult` containing statistics and any failures that
    /// occurred. Failures are individual files that could not be downloaded;
    /// the operation continues for other files.
    ///
    /// # Example
    /// ```no_run
    /// # use yashare::{YaShareClient, PublicKey, Cancel};
    /// # async fn example() -> Result<(), yashare::Error> {
    /// # let client = YaShareClient::default();
    /// # let key = PublicKey::parse("https://disk.yandex.ru/d/abc123")?;
    /// # let cancel = Cancel::new();
    /// let result = client.download_all(&key, "./downloads".as_ref(), &cancel).await?;
    ///
    /// if result.failures.is_empty() {
    ///     println!("All files downloaded successfully");
    /// } else {
    ///     println!("{} files failed", result.failures.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn download_all(
        &self,
        public_key: &PublicKey,
        dest_dir: &Path,
        cancel: &Cancel,
    ) -> Result<DownloadResult, Error> {
        let stats = Arc::new(DownloadStats::default());
        let mut failures = Vec::new();

        let aggregator = ProgressAggregator::new();
        let ctx = DownloadContext {
            progress: self.ctx.progress.with_aggregator(aggregator.clone()),
            ..self.ctx.clone()
        };
        let ticker =
            spawn_aggregate_ticker(aggregator, ctx.progress.clone(), self.aggregate_interval);

        let root_path = self.resolve_root_path(public_key, cancel).await?;
        let stream = ParallelWalker::new(
            ctx.api.clone(),
            public_key,
            self.max_listing_workers,
            cancel.clone(),
            ctx.progress.clone(),
        )
        .into_stream(root_path);
        let mut stream = Box::pin(stream);

        let pool = DownloadPool::spawn(
            self.max_concurrent_downloads,
            ctx.clone(),
            stats.clone(),
            cancel.clone(),
        );

        while let Some(item) = stream.try_next().await? {
            if cancel.check().is_err() {
                break;
            }

            if !item.is_file() {
                continue;
            }

            let path = item.path.clone().unwrap_or_default();

            match DownloadJob::for_download(dest_dir, public_key, &item) {
                Ok(job) => {
                    if pool.submit(job).await.is_err() {
                        break;
                    }
                },
                Err(err) => {
                    stats.record_failure();
                    error!(path, error = %err, "skipping item");
                    failures.push(DownloadFailure { path, error: err });
                },
            }
        }

        failures.extend(pool.join().await);

        if let Some(ticker) = ticker {
            ticker.abort();
        }

        Ok(DownloadResult { stats, failures })
    }
}
