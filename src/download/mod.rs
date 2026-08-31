pub(crate) mod concurrency;
pub(crate) mod execution;
pub(crate) mod model;
pub(crate) mod resume;
pub(crate) mod transport;

use crate::{
    api::{http::HttpClient, resource_client::ResourceClient},
    fs::checksum::VerificationMode,
    retry::policy::RetryPolicy,
};

/// Represents the context for a download session, including HTTP client, API client, and retry
/// policy.
#[derive(Clone)]
pub(crate) struct DownloadContext {
    pub(crate) http: HttpClient,
    pub(crate) api: ResourceClient,
    pub(crate) retry: RetryPolicy,
    pub(crate) max_link_attempts: usize,
    pub(crate) verify_mode: VerificationMode,
}
