use std::{path::Path, time::Duration};

use futures_util::Stream;
use reqwest::{Client, ClientBuilder, StatusCode};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    Error, Outcome,
    client::{ApiClient, DownloadClient, HttpClient, RetryPolicy, walk::Walker},
    path_safety::safe_relative_path,
    transport::{Item, Resource},
    verify::verifier_for,
};

const DEFAULT_API_BASE: &str = "https://cloud-api.yandex.net/v1/disk/public/resources";
const DEFAULT_USER_AGENT: &str = concat!("yashare/", env!("CARGO_PKG_VERSION"));
const DEFAULT_MAX_RETRIES: usize = 8;
const MAX_LINK_REFRESHES: usize = 3;

#[derive(Clone)]
pub struct ClientConfig {
    pub api_base: Url,
    pub retry_policy: RetryPolicy,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            api_base: Url::parse(DEFAULT_API_BASE).expect("default API URL is valid"),
            retry_policy: RetryPolicy::default_conditions(DEFAULT_MAX_RETRIES),
        }
    }
}

pub struct YaShareClient {
    http: HttpClient,
    api: ApiClient,
    downloader: DownloadClient,
}

impl YaShareClient {
    pub fn default() -> Result<Self, Error> {
        YaShareClientBuilder::default().build()
    }

    fn is_expired_link_error(error: &Error) -> bool {
        matches!(
            error,
            Error::Status { status, .. } | Error::Api { status, .. }
                if matches!(status, &StatusCode::FORBIDDEN | &StatusCode::GONE)
        )
    }

    pub(crate) fn from_parts(http: Client, config: ClientConfig) -> Result<Self, Error> {
        let http = HttpClient::new(http);

        let api = ApiClient::new(
            http.clone(),
            config.retry_policy.clone(),
            config.api_base.clone(),
        );

        let downloader = DownloadClient::new(http.clone(), config.retry_policy.clone());

        Ok(Self {
            http,
            api,
            downloader,
        })
    }

    pub fn set_fields(&mut self, fields: String) {
        self.api.set_fields(fields);
    }

    pub async fn resource_meta(
        &self,
        public_key: &str,
        path: Option<&str>,
        shutdown: &CancellationToken,
    ) -> Result<Resource, Error> {
        self.api
            .resource_meta(&self.normalize_key(public_key), path, shutdown)
            .await
    }

    fn normalize_key(&self, public_link: &str) -> String {
        if public_link.starts_with("http") {
            if !public_link.contains("/d/") && !public_link.contains("/public/") {
                if let Ok(url) = Url::parse(public_link)
                    && url.host_str() == Some("disk.yandex.ru")
                {
                    return public_link.to_string();
                }
                tracing::warn!("Suspicious public link: {}", public_link);
            }
            public_link.to_string()
        } else {
            format!("https://disk.yandex.ru/d/{}", public_link)
        }
    }

    pub async fn walk(
        &self,
        public_key: &str,
        shutdown: &CancellationToken,
    ) -> Result<impl Stream<Item = Result<Item, Error>>, Error> {
        let public_key = self.normalize_key(public_key);

        let root = self.api.resource_meta(&public_key, None, shutdown).await?;

        if root.type_field.as_deref() != Some("dir") {
            return Err(Error::NotAFolder(
                root.name.unwrap_or_else(|| public_key.clone()),
            ));
        }

        let root_path = root.path.unwrap_or_else(|| "/".to_string());

        Ok(Walker::new(self.api.clone(), public_key, root_path, shutdown.clone()).into_stream())
    }

    pub async fn download_item(
        &self,
        public_key: &str,
        item: &Item,
        dest_dir: &Path,
        shutdown: &CancellationToken,
    ) -> Result<Outcome, Error> {
        let public_key = self.normalize_key(public_key);

        let item_path = item
            .path
            .as_deref()
            .ok_or_else(|| Error::InvalidPath("item has no path".to_string()))?;

        let size = item
            .size
            .ok_or_else(|| Error::UnexpectedResponse("item has no size".to_string()))?;

        let relative = safe_relative_path(item_path)?;
        let destination = dest_dir.join(relative);
        let verify = verifier_for(item.checksum_spec()).await;

        let mut last_error = None;

        for refresh in 1..=MAX_LINK_REFRESHES {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let link = self
                .api
                .download_href(&public_key, Some(item_path), shutdown)
                .await?;

            match self
                .downloader
                .download(&link.href, &destination, size, verify.clone(), shutdown)
                .await
            {
                Ok(outcome) => return Ok(outcome),

                Err(err) if Self::is_expired_link_error(&err) => {
                    tracing::warn!(
                        path = item_path,
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
            path: item_path.to_string(),
        })
        .map_err(|final_err| {
            if let Some(prev) = last_error {
                tracing::debug!("last transient error before giving up: {prev}");
            }
            final_err
        })
    }

    pub async fn download_all(
        &self,
        public_key: &str,
        dest_dir: &Path,
        shutdown: &CancellationToken,
    ) -> Result<Vec<(String, Outcome)>, Error> {
        use futures_util::StreamExt;

        let public_key = self.normalize_key(public_key);
        let mut stream = Box::pin(self.walk(&public_key, shutdown).await?);
        let mut results = Vec::new();

        while let Some(item) = stream.next().await {
            let item = item?;

            // walk() отдаёт и файлы, и папки — папки пропускаем
            if item.item_type.as_deref() != Some("file") {
                continue;
            }

            let name = item.name.clone().unwrap_or_default();
            let outcome = self
                .download_item(&public_key, &item, dest_dir, shutdown)
                .await?;

            results.push((name, outcome));
        }

        Ok(results)
    }
}

pub struct YaShareClientBuilder {
    config: ClientConfig,
    http_builder: ClientBuilder,
}

impl YaShareClientBuilder {
    fn default() -> Self {
        Self {
            config: ClientConfig::default(),
            http_builder: Client::builder().user_agent(DEFAULT_USER_AGENT),
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.http_builder = self.http_builder.timeout(timeout);

        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.http_builder = self.http_builder.connect_timeout(timeout);

        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.http_builder = self.http_builder.user_agent(user_agent.into());
        self
    }

    pub fn api_base(mut self, url: Url) -> Self {
        self.config.api_base = url;
        self
    }

    pub fn api_base_str(mut self, url: impl AsRef<str>) -> Result<Self, Error> {
        self.config.api_base = Url::parse(url.as_ref())?;
        Ok(self)
    }

    pub fn retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.config.retry_policy = retry_policy;
        self
    }

    pub fn build(self) -> Result<YaShareClient, Error> {
        let http = self.http_builder.build().map_err(|_| Error::CreateClient)?;

        YaShareClient::from_parts(http, self.config)
    }
}
