use std::time::Duration;

use reqwest::{Client, ClientBuilder};
use url::Url;

use crate::{Error, YaShareClient, api::retry::RetryPolicy, checksum::VerifyMode};

const DEFAULT_API_BASE: &str = "https://cloud-api.yandex.net/v1/disk";
const DEFAULT_USER_AGENT: &str = concat!("yashare/", env!("CARGO_PKG_VERSION"));
const DEFAULT_MAX_RETRIES: usize = 8;
const DEFAULT_MAX_LINK_ATTEMPTS: usize = 3;
const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 6;
const DEFAULT_MAX_LISTERS: usize = 6;

#[derive(Clone)]
pub struct ClientConfig {
    pub api_base: Url,
    pub retry_policy: RetryPolicy,
    pub max_link_attempts: usize,
    pub max_concurrent_downloads: usize,
    pub max_concurrent_listers: usize,
    pub verify_mode: VerifyMode,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            api_base: Url::parse(DEFAULT_API_BASE).expect("default API URL is valid"),
            retry_policy: RetryPolicy::default_conditions(DEFAULT_MAX_RETRIES),
            max_link_attempts: DEFAULT_MAX_LINK_ATTEMPTS,
            max_concurrent_downloads: DEFAULT_MAX_CONCURRENT_DOWNLOADS,
            max_concurrent_listers: DEFAULT_MAX_LISTERS,
            verify_mode: VerifyMode::default(),
        }
    }
}

pub struct YaShareClientBuilder {
    config: ClientConfig,
    http_builder: ClientBuilder,
}

impl YaShareClientBuilder {
    pub fn default() -> Self {
        Self {
            config: ClientConfig::default(),
            http_builder: Client::builder().user_agent(DEFAULT_USER_AGENT),
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_link_attempts(mut self, max_link_attempts: usize) -> Self {
        self.config.max_link_attempts = max_link_attempts;
        self
    }

    pub fn max_concurrent_downloads(mut self, max_concurrent_downloads: usize) -> Self {
        self.config.max_concurrent_downloads = max_concurrent_downloads.max(1);
        self
    }

    pub fn max_concurrent_listers(mut self, max_listers: usize) -> Self {
        self.config.max_concurrent_listers = max_listers.max(1);
        self
    }

    pub fn verify_mode(mut self, verify_mode: VerifyMode) -> Self {
        self.config.verify_mode = verify_mode;
        self
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
        if self.config.max_link_attempts == 0 {
            return Err(Error::InvalidMaxLinkAttempts(self.config.max_link_attempts));
        }
        let http = self.http_builder.build().map_err(|_| Error::CreateClient)?;

        YaShareClient::from_parts(http, self.config)
    }
}
