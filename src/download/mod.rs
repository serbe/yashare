mod job;
mod link_provider;
mod pool;
mod result;
mod resume;
mod session;
mod stats;
mod worker;

pub(crate) use job::DownloadJob;
pub(crate) use link_provider::DownloadLinkProvider;
pub(crate) use pool::DownloadPool;
pub use result::DownloadResult;
pub(crate) use resume::{ResumeAction, ResumeManager, ResumeState};
pub(crate) use session::DownloadSession;
pub use stats::{DownloadFailure, DownloadStats};
pub(crate) use worker::DownloadWorker;
pub use worker::Outcome;

use crate::{
    api::{HttpClient, ResourceClient},
    fs::VerificationMode,
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
