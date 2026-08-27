pub mod builder;

use std::{path::Path, sync::Arc};

use futures_util::{Stream, TryStreamExt};
use reqwest::Client;
use tokio_util::sync::CancellationToken;

pub use builder::{ClientConfig, YaShareClientBuilder};

use crate::{
    Error,
    api::{http::HttpClient, resource_client::ResourceClient},
    download::{
        DownloadContext,
        job::DownloadJob,
        pool::DownloadPool,
        stats::{DownloadFailure, DownloadResult, DownloadStats},
        worker::{DownloadWorker, Outcome},
    },
    model::{Item, Resource},
    public_key::PublicKey,
    walker::{parallel::ParallelWalker, sequential::Walker},
};

#[derive(Clone)]
pub struct YaShareClient {
    ctx: DownloadContext,
    max_concurrent_downloads: usize,
    max_listing_workers: usize,
}

impl Default for YaShareClient {
    fn default() -> Self {
        YaShareClientBuilder::default()
            .build()
            .expect("failed to create default client")
    }
}

impl YaShareClient {
    pub(crate) fn from_parts(http: Client, config: ClientConfig) -> Result<Self, Error> {
        let http = HttpClient::new(http);

        let api = ResourceClient::new(
            http.clone(),
            config.retry_policy.clone(),
            config.api_base.clone(),
        );

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

    pub fn set_fields(&mut self, fields: String) {
        self.ctx.api.set_fields(fields);
    }

    async fn resolve_root_path(
        &self,
        public_key: &PublicKey,
        shutdown: &CancellationToken,
    ) -> Result<String, Error> {
        let root = self
            .ctx
            .api
            .resource_meta(public_key, None, shutdown)
            .await?;

        if !root.is_dir() {
            return Err(Error::NotAFolder(
                root.name.unwrap_or_else(|| public_key.as_api_string()),
            ));
        }

        Ok(root.path.unwrap_or_else(|| "/".to_string()))
    }

    pub async fn resource_meta(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        shutdown: &CancellationToken,
    ) -> Result<Resource, Error> {
        self.ctx.api.resource_meta(public_key, path, shutdown).await
    }

    pub async fn walk(
        &self,
        public_key: &PublicKey,
        shutdown: &CancellationToken,
    ) -> Result<impl Stream<Item = Result<Item, Error>>, Error> {
        let root_path = self.resolve_root_path(public_key, shutdown).await?;

        Ok(Walker::new(
            self.ctx.api.clone(),
            public_key,
            root_path,
            shutdown.clone(),
        )
        .into_stream())
    }

    pub async fn download_item(
        &self,
        public_key: &PublicKey,
        item: &Item,
        dest_dir: &Path,
        shutdown: &CancellationToken,
    ) -> Result<Outcome, Error> {
        let job = DownloadJob::for_download(dest_dir, public_key, item)?;

        let mut worker = DownloadWorker::single(self.ctx.clone());

        worker.download_job(job, shutdown).await
    }

    pub async fn download_all(
        &self,
        public_key: &PublicKey,
        dest_dir: &Path,
        shutdown: &CancellationToken,
    ) -> Result<DownloadResult, Error> {
        let stats = Arc::new(DownloadStats::default());
        let mut failures = Vec::new();

        let root_path = self.resolve_root_path(public_key, shutdown).await?;
        let stream = ParallelWalker::new(
            self.ctx.api.clone(),
            public_key,
            self.max_listing_workers,
            shutdown.clone(),
        )
        .into_stream(root_path);
        let mut stream = Box::pin(stream);

        let pool = DownloadPool::spawn(
            self.max_concurrent_downloads,
            self.ctx.clone(),
            stats.clone(),
            shutdown.clone(),
        );

        while let Some(item) = stream.try_next().await? {
            if shutdown.is_cancelled() {
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
                }
                Err(err) => {
                    stats.record_failure();
                    tracing::error!(path, error = %err, "skipping item");
                    failures.push(DownloadFailure { path, error: err });
                }
            }
        }

        failures.extend(pool.join().await);

        Ok(DownloadResult { stats, failures })
    }
}
