mod job;
mod outcome;
mod result;
mod stats;

pub(crate) use job::DownloadJob;
pub(crate) use outcome::Outcome;
pub(crate) use result::DownloadResult;
pub(crate) use stats::{DownloadFailure, DownloadStats};
