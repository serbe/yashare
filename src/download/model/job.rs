use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    Error,
    error::{ApiError, ClientError},
    fs::{ChecksumSpec, safe_relative_path},
    model::{Item, PublicKey},
};

/// Global monotonic counter for generating unique job identifiers.
///
/// Job IDs serve two purposes:
/// - Correlating `ProgressEvent`s emitted for the same download within a process.
/// - Providing a stable identifier even when a job is retried multiple times (link expiry, retry
///   policy, etc.).
///
/// # Invariants
/// The counter starts at 1 and increments by 1 for each new job. It never
/// decreases. Job IDs are not persisted across process restarts — they are
/// purely runtime identifiers.
static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Generates the next unique job ID.
///
/// This is lock-free and uses relaxed ordering because the only requirement
/// is that IDs are unique; there is no dependency between IDs and no need
/// for strong ordering guarantees.
fn next_job_id() -> u64 {
    NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed)
}

/// A complete description of a file to download.
///
/// `DownloadJob` contains all the information needed by a download worker
/// to download a single file: where to get it, where to put it, and how to
/// verify it.
///
/// # Memory optimization
/// `item_path` is stored as `Arc<str>` rather than `String`. This allows
/// cheap cloning into every progress event (`Started`/`Finished`/`Failed`)
/// without allocating a new string for each event. Given that large
/// directory walks can generate thousands of events, this reduces heap
/// pressure significantly.
#[derive(Debug, Clone)]
pub struct DownloadJob {
    /// Unique identifier for this job, used only for progress event
    /// correlation. Not persisted across runs.
    pub job_id: u64,
    /// The public key identifying the shared resource.
    pub public_key: PublicKey,
    /// Remote path of the file within the share.
    ///
    /// This is an `Arc<str>` to allow cheap cloning into progress events
    /// without reallocation.
    pub item_path: Arc<str>,
    /// Local filesystem path where the file should be written.
    pub destination: PathBuf,
    /// Expected size of the file in bytes.
    pub size: u64,
    /// Checksum specification for integrity verification.
    pub checksum: ChecksumSpec,
    /// Optional pre-known download URL.
    ///
    /// If `Some`, the download worker will try this URL first before
    /// requesting a fresh one from the API. This avoids an extra API
    /// call when the link is still valid.
    pub initial_href: Option<String>,
}

impl DownloadJob {
    /// Creates a download job for an item in a public share.
    ///
    /// This is the primary constructor used during directory walks. It:
    /// 1. Extracts the remote path from the item.
    /// 2. Converts the remote path to a safe local path relative to `dest_dir`.
    /// 3. Packages all metadata into a `DownloadJob`.
    ///
    /// # Path safety
    /// The remote path is sanitized using `safe_relative_path` to prevent
    /// directory traversal attacks and to handle Windows-incompatible
    /// characters.
    ///
    /// # Errors
    /// Returns `ClientError::InvalidPath` if the item has no path or the
    /// path is invalid.
    pub(crate) fn for_download(
        dest_dir: &Path,
        public_key: &PublicKey,
        item: &Item,
    ) -> Result<Self, Error> {
        let item_path = item.path.as_deref().ok_or_else(|| {
            Error::Client(ClientError::InvalidPath("item has no path".to_string()))
        })?;
        let destination = dest_dir.join(safe_relative_path(item_path)?);
        Self::from_item(public_key, item, destination)
    }

    /// Creates a download job from an item and a pre-determined destination.
    ///
    /// This is useful when the caller already knows the exact destination
    /// path (e.g., from a previous run or a custom mapping).
    ///
    /// # Errors
    /// Returns an error if the item lacks a path or size.
    pub(crate) fn from_item(
        public_key: &PublicKey,
        item: &Item,
        destination: PathBuf,
    ) -> Result<Self, Error> {
        let item_path = item.path.clone().ok_or_else(|| {
            Error::Client(ClientError::InvalidPath("item has no path".to_string()))
        })?;

        let size = item.size.ok_or_else(|| {
            Error::Api(ApiError::UnexpectedResponse("item has no size".to_string()))
        })?;

        Ok(Self {
            job_id: next_job_id(),
            public_key: public_key.clone(),
            item_path: Arc::from(item_path),
            destination,
            size,
            checksum: item.checksum_spec(),
            initial_href: item.file.clone(),
        })
    }
}
