use std::{path::Path, sync::Arc};

use dashmap::DashSet;
use futures_util::{Stream, TryStreamExt};
use reqwest::Client;
use tokio_util::sync::CancellationToken;

use crate::{
    Error,
    api::{api::ApiClient, http_client::HttpClient, retry::RetryPolicy},
    checksum::VerifyMode,
    client::builder::{ClientConfig, YaShareClientBuilder},
    download::{
        job::DownloadJob,
        pool::DownloadPool,
        stats::{DownloadFailure, DownloadResult, DownloadStats},
        worker::{DownloadWorker, Outcome},
    },
    model::{Item, Resource},
    path_safety::safe_relative_path,
    utils::PublicKey,
    walker::{parallel::ParallelWalker, sequential::Walker},
};

#[derive(Clone)]
pub struct YaShareClient {
    http: HttpClient,
    api: ApiClient,
    retry: RetryPolicy,
    max_link_attempts: usize,
    max_concurrent_downloads: usize,
    max_concurrent_listers: usize,
    verify_mode: VerifyMode,
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

        let api = ApiClient::new(
            http.clone(),
            config.retry_policy.clone(),
            config.api_base.clone(),
        );

        Ok(Self {
            http,
            api,
            retry: config.retry_policy,
            max_link_attempts: config.max_link_attempts,
            max_concurrent_downloads: config.max_concurrent_downloads,
            max_concurrent_listers: config.max_concurrent_listers,
            verify_mode: config.verify_mode,
        })
    }

    pub fn set_fields(&mut self, fields: String) {
        self.api.set_fields(fields);
    }

    async fn resolve_root_path(
        &self,
        public_key: &PublicKey,
        shutdown: &CancellationToken,
    ) -> Result<String, Error> {
        let root = self.api.resource_meta(public_key, None, shutdown).await?;

        if root.type_field.as_deref() != Some("dir") {
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
        self.api.resource_meta(public_key, path, shutdown).await
    }

    pub async fn walk(
        &self,
        public_key: &PublicKey,
        shutdown: &CancellationToken,
    ) -> Result<impl Stream<Item = Result<Item, Error>>, Error> {
        let root = self.api.resource_meta(public_key, None, shutdown).await?;

        if root.type_field.as_deref() != Some("dir") {
            return Err(Error::NotAFolder(
                root.name.unwrap_or_else(|| public_key.as_api_string()),
            ));
        }

        let root_path = root.path.unwrap_or_else(|| "/".to_string());

        Ok(Walker::new(self.api.clone(), public_key, root_path, shutdown.clone()).into_stream())
    }

    fn build_job(
        dest_dir: &Path,
        public_key: &PublicKey,
        item: &Item,
    ) -> Result<DownloadJob, Error> {
        let item_path = item
            .path
            .as_deref()
            .ok_or_else(|| Error::InvalidPath("item has no path".to_string()))?;

        let relative = safe_relative_path(item_path)?;
        let destination = dest_dir.join(relative);

        DownloadJob::from_item(public_key, item, destination)
    }

    pub async fn download_item(
        &self,
        public_key: &PublicKey,
        item: &Item,
        dest_dir: &Path,
        shutdown: &CancellationToken,
    ) -> Result<Outcome, Error> {
        let job = Self::build_job(dest_dir, public_key, item)?;

        let mut worker = DownloadWorker::new(
            0,
            self.http.clone(),
            self.api.clone(),
            self.retry.clone(),
            self.max_link_attempts,
            Arc::new(DashSet::new()),
            self.verify_mode,
        );

        worker.process(job, shutdown).await
    }

    pub async fn download_all(
        &self,
        public_key: &PublicKey,
        dest_dir: &Path,
        shutdown: &CancellationToken,
    ) -> Result<DownloadResult, Error> {
        let stats = Arc::new(DownloadStats::default());
        let mut failures = Vec::new();

        // let mut stream = Box::pin(self.walk(public_key, shutdown).await?);
        let root_path = self.resolve_root_path(public_key, shutdown).await?;
        let stream = ParallelWalker::new(
            self.api.clone(),
            public_key,
            self.max_concurrent_listers,
            shutdown.clone(),
        )
        .listers(16) // подберите под свою нагрузку
        .into_stream(root_path);
        let mut stream = Box::pin(stream);

        let pool = DownloadPool::spawn(
            self.max_concurrent_downloads,
            self.http.clone(),
            self.api.clone(),
            self.retry.clone(),
            self.max_link_attempts,
            self.verify_mode,
            stats.clone(),
            shutdown.clone(),
        );

        while let Some(item) = stream.try_next().await? {
            if shutdown.is_cancelled() {
                break;
            }

            if item.item_type.as_deref() != Some("file") {
                continue;
            }

            let path = item.path.clone().unwrap_or_default();

            match Self::build_job(dest_dir, public_key, &item) {
                Ok(job) => {
                    if pool.submit(job).await.is_err() {
                        // канал закрыт — воркеры уже остановились
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
