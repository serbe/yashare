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
        concurrency::DownloadPool,
        execution::DownloadWorker,
        model::{DownloadFailure, DownloadJob, DownloadResult, DownloadStats, Outcome},
    },
    error::ClientError,
    model::{Item, PublicKey, Resource},
    walker::{ParallelWalker, Walker},
};

/// The main client for interacting with Yandex.Disk public shares.
#[derive(Clone)]
pub struct YaShareClient {
    ctx: DownloadContext,
    max_concurrent_downloads: usize,
    max_listing_workers: usize,
}

impl Default for YaShareClient {
    /// Creates a default [`YaShareClient`] using the [`YaShareClientBuilder`] with default
    /// configuration.
    fn default() -> Self {
        YaShareClientBuilder::default()
            .build()
            .expect("failed to create default client")
    }
}

impl YaShareClient {
    /// Creates a [`YaShareClient`] from the HTTP client and configuration.
    pub(crate) fn from_parts(http: Client, config: ClientConfig) -> Result<Self, Error> {
        let http = HttpClient::new(http);

        let api =
            ResourceClient::new(http.clone(), config.retry_policy.clone(), config.api_base.clone());

        let ctx = DownloadContext {
            http,
            api,
            retry: config.retry_policy,
            max_link_attempts: config.max_link_attempts,
            verify_mode: config.verify_mode,
        };

        Ok(Self {
            ctx,
            max_concurrent_downloads: config.max_concurrent_downloads,
            max_listing_workers: config.max_listing_workers,
        })
    }

    /// Sets the fields to request from the API for resource metadata.
    pub fn set_fields(&mut self, fields: String) {
        self.ctx.api.set_fields(fields);
    }

    /// Resolves the root path of a public folder.
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
    pub async fn resource_meta(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Resource, Error> {
        self.ctx.api.resource_meta(public_key, path, cancel).await
    }

    /// Returns a stream of all items in a public folder (recursive).
    pub async fn walk(
        &self,
        public_key: &PublicKey,
        cancel: &Cancel,
    ) -> Result<impl Stream<Item = Result<Item, Error>>, Error> {
        let root_path = self.resolve_root_path(public_key, cancel).await?;

        Ok(Walker::new(self.ctx.api.clone(), public_key, root_path, cancel.clone()).into_stream())
    }

    /// Downloads a single item from the API to the specified directory.
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
    pub async fn download_all(
        &self,
        public_key: &PublicKey,
        dest_dir: &Path,
        cancel: &Cancel,
    ) -> Result<DownloadResult, Error> {
        let stats = Arc::new(DownloadStats::default());
        let mut failures = Vec::new();

        let root_path = self.resolve_root_path(public_key, cancel).await?;
        let stream = ParallelWalker::new(
            self.ctx.api.clone(),
            public_key,
            self.max_listing_workers,
            cancel.clone(),
        )
        .into_stream(root_path);
        let mut stream = Box::pin(stream);

        let pool = DownloadPool::spawn(
            self.max_concurrent_downloads,
            self.ctx.clone(),
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

        Ok(DownloadResult { stats, failures })
    }
}
