mod concurrency;
mod execution;
mod model;
mod resume;
mod transport;

use crate::{
    api::{HttpClient, ResourceClient},
    fs::VerificationMode,
    retry::RetryPolicy,
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
