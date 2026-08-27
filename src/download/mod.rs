pub(crate) mod job;
pub(crate) mod path_safety;
pub(crate) mod pool;
pub(crate) mod resume;
pub(crate) mod stats;
pub(crate) mod verification;
pub(crate) mod worker;

use crate::{
    api::{http::HttpClient, resource_client::ResourceClient, retry::RetryPolicy},
    checksum::VerificationMode,
};

#[derive(Clone)]
pub(crate) struct DownloadContext {
    pub(crate) http: HttpClient,
    pub(crate) api: ResourceClient,
    pub(crate) retry: RetryPolicy,
    pub(crate) max_link_attempts: usize,
    pub(crate) verify_mode: VerificationMode,
}
