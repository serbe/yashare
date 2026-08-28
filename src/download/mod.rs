mod job;
mod path_safety;
mod pool;
mod resume;
mod stats;
mod verification;
mod worker;

pub(crate) use job::DownloadJob;
pub(crate) use pool::DownloadPool;
pub(crate) use stats::{DownloadFailure, DownloadResult, DownloadStats};
pub(crate) use worker::{DownloadWorker, Outcome};

use crate::{
    api::{HttpClient, ResourceClient},
    checksum::VerificationMode,
    retry::RetryPolicy,
};

#[derive(Clone)]
pub(crate) struct DownloadContext {
    pub(crate) http: HttpClient,
    pub(crate) api: ResourceClient,
    pub(crate) retry: RetryPolicy,
    pub(crate) max_link_attempts: usize,
    pub(crate) verify_mode: VerificationMode,
}
