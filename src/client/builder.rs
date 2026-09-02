use std::time::Duration;

use reqwest::{Client, ClientBuilder};
use url::Url;

use crate::{
    Error, client::YaShareClient, download::progress::ProgressEvent, error::HttpError,
    fs::VerificationMode, retry::RetryPolicy,
};

/// Default Yandex.Disk API base URL.
const DEFAULT_API_BASE: &str = "https://cloud-api.yandex.net/v1/disk";

/// Default user agent string for HTTP requests.
///
/// Format: `yashare/{version}` where version is the crate version from Cargo.toml.
const DEFAULT_USER_AGENT: &str = concat!("yashare/", env!("CARGO_PKG_VERSION"));

/// Default maximum number of retry attempts for API requests.
const DEFAULT_MAX_RETRIES: usize = 8;

/// Default maximum number of attempts to obtain a fresh download link.
const DEFAULT_MAX_LINK_ATTEMPTS: usize = 3;

/// Default maximum number of concurrent downloads.
const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 6;

/// Default maximum number of listing workers for parallel directory walks.
const DEFAULT_MAX_LISTING_WORKERS: usize = 6;

/// Default interval between aggregate progress snapshots.
const DEFAULT_AGGREGATE_INTERVAL: Duration = Duration::from_millis(200);

/// Configuration for the [`YaShareClient`].
///
/// This struct holds all configurable parameters for the client. It is
/// constructed via `YaShareClientBuilder` and consumed when building the
/// client.
#[derive(Clone)]
pub struct ClientConfig {
    pub api_base: Url,
    pub retry_policy: RetryPolicy,
    pub max_link_attempts: usize,
    pub max_concurrent_downloads: usize,
    pub max_listing_workers: usize,
    pub verify_mode: VerificationMode,
    pub progress_sender: Option<async_channel::Sender<ProgressEvent>>,
    pub aggregate_interval: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            api_base: Url::parse(DEFAULT_API_BASE).expect("default API URL is valid"),
            retry_policy: RetryPolicy::default_conditions(DEFAULT_MAX_RETRIES.max(1)),
            max_link_attempts: DEFAULT_MAX_LINK_ATTEMPTS.max(1),
            max_concurrent_downloads: DEFAULT_MAX_CONCURRENT_DOWNLOADS.max(1),
            max_listing_workers: DEFAULT_MAX_LISTING_WORKERS.max(1),
            verify_mode: VerificationMode::default(),
            progress_sender: None,
            aggregate_interval: DEFAULT_AGGREGATE_INTERVAL,
        }
    }
}

/// Builder for creating a [`YaShareClient`] with custom configuration.
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
///
/// # Progress reporting
/// To receive progress events, call `progress_sender()` with a channel
/// sender. The library sends events without blocking, so slow receivers
/// may drop events. Use an unbounded channel if you need to guarantee
/// delivery of every event.
pub struct YaShareClientBuilder {
    config: ClientConfig,
    http_builder: ClientBuilder,
}

impl Default for YaShareClientBuilder {
    fn default() -> Self {
        Self {
            config: ClientConfig::default(),
            http_builder: Client::builder().user_agent(DEFAULT_USER_AGENT),
        }
    }
}

impl YaShareClientBuilder {
    /// Creates a new builder with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum number of attempts to obtain a fresh download link.
    ///
    /// Download links from Yandex.Disk expire after a few hours. This setting
    /// controls how many times the client will request a new link when the
    /// current one expires during a download. Default: 3.
    ///
    /// # Minimum
    /// The value is clamped to at least 1.
    pub fn max_link_attempts(mut self, max_link_attempts: usize) -> Self {
        self.config.max_link_attempts = max_link_attempts.max(1);
        self
    }

    /// Sets the maximum number of concurrent downloads.
    ///
    /// This controls how many files are downloaded simultaneously. Higher
    /// values can improve throughput but may increase memory usage and
    /// network congestion. Default: 6.
    ///
    /// # Minimum
    /// The value is clamped to at least 1.
    pub fn max_concurrent_downloads(mut self, max_concurrent_downloads: usize) -> Self {
        self.config.max_concurrent_downloads = max_concurrent_downloads.max(1);
        self
    }

    /// Sets the maximum number of workers for parallel directory listing.
    ///
    /// When walking a directory tree, the client can use multiple workers
    /// to list directories concurrently. This setting controls the maximum
    /// number of concurrent listing requests. Default: 6.
    ///
    /// # Minimum
    /// The value is clamped to at least 1.
    pub fn max_listing_workers(mut self, max_listing_workers: usize) -> Self {
        self.config.max_listing_workers = max_listing_workers.max(1);
        self
    }

    /// Sets the verification mode for downloaded files.
    ///
    /// - `VerificationMode::SizeOnly`: Only check file size (faster).
    /// - `VerificationMode::SizeAndChecksum`: Check size and checksum (more reliable but slower for
    ///   large files).
    ///
    /// Default: `SizeOnly`.
    pub fn verify_mode(mut self, verify_mode: VerificationMode) -> Self {
        self.config.verify_mode = verify_mode;
        self
    }

    /// Sets the timeout for the HTTP client.
    ///
    /// This is the total timeout from the start of a request to the receipt
    /// of the complete response. It includes connection establishment,
    /// request sending, and response reading.
    ///
    /// Default: None (no timeout).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.http_builder = self.http_builder.timeout(timeout);

        self
    }

    /// Sets the connect timeout for the HTTP client.
    ///
    /// This is the maximum time allowed for establishing a connection to the
    /// server. It does not include the time to send the request or read the
    /// response.
    ///
    /// Default: None (no timeout).
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.http_builder = self.http_builder.connect_timeout(timeout);

        self
    }

    /// Sets the User-Agent header for HTTP requests.
    ///
    /// Default: `yashare/{version}`.
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.http_builder = self.http_builder.user_agent(user_agent.into());
        self
    }

    /// Sets the API base URL.
    ///
    /// This is useful for using a custom or development API endpoint.
    /// Default: `https://cloud-api.yandex.net/v1/disk`.
    pub fn api_base(mut self, url: Url) -> Self {
        self.config.api_base = url;
        self
    }

    /// Sets the API base URL from a string.
    ///
    /// This is a convenience method that parses the URL string. Returns an
    /// error if the URL is invalid.
    pub fn api_base_str(mut self, url: impl AsRef<str>) -> Result<Self, Error> {
        self.config.api_base = Url::parse(url.as_ref())?;
        Ok(self)
    }

    /// Sets the retry policy for the client.
    ///
    /// The retry policy controls how the client handles transient errors
    /// (rate limiting, network issues, service unavailability).
    ///
    /// Default: Exponential backoff with up to 8 attempts.
    pub fn retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.config.retry_policy = retry_policy;
        self
    }

    /// Registers a channel sender that will receive [`ProgressEvent`]s.
    ///
    /// Events emitted:
    /// - Discovery events while the remote tree is being walked.
    /// - Per-job download events (`Started`, `Progress`, `Finished`, `Failed`).
    /// - Periodic aggregate snapshots.
    ///
    /// # Delivery semantics
    /// The library never blocks on this channel — events are sent with
    /// `try_send`. A slow or full receiver simply misses events rather than
    /// slowing down downloads.
    ///
    /// # Channel choice
    /// Use an unbounded channel (`async_channel::unbounded`) if you don't
    /// want to lose events under load, since `try_send` on an unbounded
    /// channel never fails due to capacity. Use a bounded channel if you
    /// want to bound memory usage and are willing to drop events.
    ///
    /// # Example
    /// ```no_run
    /// # async fn example() -> Result<(), yashare::Error> {
    /// let (tx, rx) = async_channel::unbounded();
    ///
    /// let client = yashare::YaShareClient::builder().progress_sender(tx).build()?;
    ///
    /// tokio::spawn(async move {
    ///     while let Ok(event) = rx.recv().await {
    ///         println!("{event:?}");
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub fn progress_sender(mut self, sender: async_channel::Sender<ProgressEvent>) -> Self {
        self.config.progress_sender = Some(sender);
        self
    }

    /// Sets how often `ProgressEvent::Aggregate` snapshots are emitted.
    ///
    /// This setting has no effect unless a `progress_sender` is configured.
    /// Default: 200ms.
    pub fn aggregate_interval(mut self, interval: Duration) -> Self {
        self.config.aggregate_interval = interval;
        self
    }

    /// Builds the [`YaShareClient`] from the configured settings.
    ///
    /// # Errors
    /// Returns `Error::Http(HttpError::CreateClient)` if the HTTP client
    /// cannot be created (e.g., due to invalid configuration).
    pub fn build(self) -> Result<YaShareClient, Error> {
        let http = self.http_builder.build().map_err(|_| Error::Http(HttpError::CreateClient))?;

        YaShareClient::from_parts(http, self.config)
    }
}
