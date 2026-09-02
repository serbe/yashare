use std::path::Path;

use crate::{
    api::HttpClient,
    download::{
        DownloadContext, progress::ProgressEmitter, resume::ResumeManager,
        transport::session::DownloadSession,
    },
    fs::{ChecksumSpec, FileVerifier, VerificationMode},
};

/// Size of each chunk used for streaming file verification.
///
/// A 1 MiB buffer balances memory usage against I/O overhead — large enough
/// to amortize syscall costs, small enough to keep per-worker memory
/// bounded even under high concurrency.
const CHUNK_SIZE: usize = 1024 * 1024;

/// Factory for creating short-lived download sessions.
///
/// Owns the long-lived resources that are shared across downloads within a
/// single worker: HTTP client, resume state manager, and file verifier with
/// its internal buffer. Each worker gets its own factory, ensuring that
/// verification state is never contended between concurrent downloads.
///
/// The factory is designed to be used in a single-threaded worker context:
/// `session()` takes `&mut self`, so only one session can be active at a
/// time. This matches the worker's execution model, where downloads are
/// processed sequentially (though the pool as a whole is concurrent).
///
/// # Invariants
/// - The verifier's buffer is reused across sessions within the same worker, avoiding reallocation
///   for each file.
/// - The resume manager tracks partial files on disk; it is stateless between calls and relies on
///   filesystem state.
pub(crate) struct SessionFactory {
    http: HttpClient,
    resume: ResumeManager,
    verifier: FileVerifier,
    verify_mode: VerificationMode,
    progress: ProgressEmitter,
}

impl SessionFactory {
    /// Creates a session factory from a shared download context.
    ///
    /// This clones the HTTP client and progress emitter (both cheap to
    /// clone) but moves the verifier and resume manager, making them owned
    /// exclusively by this factory. Each worker should have its own
    /// factory to avoid contention on the verifier's internal buffer.
    ///
    /// # Performance
    /// The file verifier allocates its `CHUNK_SIZE` buffer on creation.
    /// Creating many factories in quick succession may temporarily increase
    /// memory usage, but in practice the number of workers is bounded by
    /// `max_concurrent_downloads`.
    pub(crate) fn new(ctx: &DownloadContext) -> Self {
        Self {
            http: ctx.http.clone(),
            resume: ResumeManager::new(),
            verifier: FileVerifier::new(CHUNK_SIZE),
            verify_mode: ctx.verify_mode,
            progress: ctx.progress.clone(),
        }
    }

    /// Borrows a `DownloadSession` for a single download attempt.
    ///
    /// The session holds mutable references to the verifier (so it can
    /// update its internal state while streaming) and the resume manager.
    /// Because it takes `&mut self`, only one session can be active per
    /// factory at a time — exactly the usage pattern of `DownloadWorker`.
    ///
    /// # Lifetime
    /// The returned session is tied to the factory's lifetime. Drop the
    /// session after the download completes to release the mutable borrow.
    pub(crate) fn session(&mut self) -> DownloadSession<'_> {
        DownloadSession::new(
            &self.http,
            &self.resume,
            &mut self.verifier,
            self.verify_mode,
            &self.progress,
        )
    }

    /// Checks whether an existing file at `destination` matches the expected
    /// size and optional checksum, without performing a download.
    ///
    /// This is used during the pre-download phase to skip files that are
    /// already complete. The verification mode controls whether checksums
    /// are checked or only size.
    ///
    /// # I/O Behavior
    /// This reads the file from disk and may compute a checksum, which for
    /// large files could be expensive. However, this is only called once per
    /// file before starting a download, so the cost is amortized over the
    /// download time.
    ///
    /// # Errors
    /// Returns `std::io::Result` rather than `crate::Error` because this is
    /// called during pre-flight checks and the caller handles I/O errors by
    /// treating the file as missing (which triggers a fresh download).
    pub(crate) async fn file_matches(
        &mut self,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
    ) -> std::io::Result<bool> {
        self.verifier
            .file_matches(destination, expected_size, checksum, self.verify_mode)
            .await
    }
}
