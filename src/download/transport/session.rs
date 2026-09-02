use std::{path::Path, sync::Arc};

use futures_util::StreamExt;
use reqwest::{Response, header::HeaderMap};
use tokio::{
    fs::{remove_file, rename, try_exists},
    io::AsyncWriteExt,
};
use tracing::warn;

use crate::{
    Error,
    api::{HttpClient, map_download_error},
    cancel::Cancel,
    download::{
        model::Outcome,
        progress::{ProgressEmitter, ProgressEvent},
        resume::{ResumeAction, ResumeManager, ResumeState},
    },
    error::HttpError,
    fs::{ChecksumSpec, FileVerifier, VerificationMode},
    io_error,
};

/// Manages a single download of one file, including resumption and integrity
/// verification.
///
/// A session encapsulates the entire lifecycle of downloading a single file:
///
/// 1. Inspect existing partial file (if any) to determine whether to start fresh, resume, or
///    finalize.
/// 2. Send an HTTP request with appropriate `Range` headers for resumption.
/// 3. Stream the response body to disk, emitting progress events.
/// 4. Verify the final file's size and checksum.
/// 5. Rename the temporary `.part` file to its final destination.
///
/// # Progress events
/// Sessions emit `Started`, `Progress`, and `Finished` events on success.
/// Notably, they deliberately do **not** emit `Failed` events — that
/// responsibility belongs to `DownloadWorker`, which has the context to
/// decide whether a failure is retryable or final.
///
/// # Error recovery
/// When resume attempts fail due to a mismatched `Content-Range` or a
/// `416 Range Not Satisfiable` response, the session discards the partial
/// file and starts from scratch. This is the only self-recovery behavior
/// at the session level; all other errors are propagated to the caller.
pub(crate) struct DownloadSession<'a> {
    http: &'a HttpClient,
    resume: &'a ResumeManager,
    verifier: &'a mut FileVerifier,
    verify_mode: VerificationMode,
    progress: &'a ProgressEmitter,
}

impl<'a> DownloadSession<'a> {
    /// Creates a new download session with the given dependencies.
    ///
    /// All parameters are borrowed for the duration of the session. The
    /// verifier is borrowed mutably because it reuses an internal buffer
    /// across all verification operations within the session.
    pub(crate) fn new(
        http: &'a HttpClient,
        resume: &'a ResumeManager,
        verifier: &'a mut FileVerifier,
        verify_mode: VerificationMode,
        progress: &'a ProgressEmitter,
    ) -> Self {
        Self {
            http,
            resume,
            verifier,
            verify_mode,
            progress,
        }
    }

    /// Executes the full download lifecycle for a single file.
    ///
    /// # Arguments
    /// - `job_id`: Identifier for correlating progress events.
    /// - `item_path`: Remote path of the file, used only for progress events.
    /// - `url`: Download URL (must be a direct, authenticated URL).
    /// - `destination`: Where to write the final file.
    /// - `expected_size`: Expected file size in bytes.
    /// - `checksum`: Expected checksum (may be `None` if not verifying).
    /// - `cancel`: Cancellation token.
    ///
    /// # Returns
    /// `Outcome::AlreadyComplete` if the file already exists and matches
    /// the expected size/checksum. `Outcome::Resumed` if the download was
    /// resumed from a partial file. `Outcome::Downloaded` if downloaded
    /// from scratch.
    ///
    /// # Errors
    /// Returns `Error` for network failures, I/O errors, size mismatches,
    /// checksum mismatches, or cancellation.
    pub async fn run(
        &mut self,
        job_id: u64,
        item_path: &Arc<str>,
        url: &str,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        let mut state = self.resume.inspect(destination, expected_size).await?;

        if matches!(state.action, ResumeAction::Finalize) {
            self.finalize_existing_part(&state, destination, checksum).await?;
            self.emit_started_and_finished(
                job_id,
                item_path,
                expected_size,
                Outcome::AlreadyComplete,
            );
            return Ok(Outcome::AlreadyComplete);
        }

        if let ResumeAction::Restart { corrupt_size } = state.action {
            warn!(
                path = %state.part_path.display(),
                corrupt_size,
                expected_size,
                "partial file larger than expected or server rejected Range, restarting from scratch",
            );
            self.resume.reset(&state).await?;
            state = self.resume.inspect(destination, expected_size).await?;
        }

        if self.progress.is_active() {
            let starting_offset = state.offset();
            self.progress.emit(ProgressEvent::Started {
                job_id,
                path: item_path.clone(),
                total_size: expected_size,
            });
            if starting_offset > 0 {
                self.progress.emit(ProgressEvent::Progress {
                    job_id,
                    bytes_written: starting_offset,
                    total_size: expected_size,
                });
            }
        }

        let (response, state) = self.send_request(url, state, expected_size, destination).await?;

        self.download_body(job_id, response, &state, expected_size, cancel).await?;

        let outcome =
            self.verify_and_finalize(destination, &state, expected_size, checksum).await?;

        if self.progress.is_active() {
            self.progress.emit(ProgressEvent::Finished {
                job_id,
                path: item_path.clone(),
                outcome,
            });
        }

        Ok(outcome)
    }

    /// Emits synthetic `Started` and `Finished` events for a file that was
    /// already complete.
    ///
    /// This gives observers a complete event stream even for skipped files,
    /// so they don't need special-case handling for the "no download needed"
    /// scenario.
    fn emit_started_and_finished(
        &self,
        job_id: u64,
        item_path: &Arc<str>,
        total_size: u64,
        outcome: Outcome,
    ) {
        if !self.progress.is_active() {
            return;
        }
        self.progress.emit(ProgressEvent::Started {
            job_id,
            path: item_path.clone(),
            total_size,
        });
        self.progress
            .emit(ProgressEvent::Finished { job_id, path: item_path.clone(), outcome });
    }

    /// Sends the HTTP request with appropriate range headers.
    ///
    /// If resuming, this adds a `Range: bytes=N-` header. The response is
    /// validated to ensure the server honors the range request (status 206
    /// with matching `Content-Range`).
    ///
    /// # Recovery
    /// If the server returns `416 Range Not Satisfiable` or an unexpected
    /// `Content-Range`, the partial file is discarded and the error is
    /// propagated so the caller can retry from scratch.
    async fn send_request(
        &self,
        url: &str,
        state: ResumeState,
        expected_size: u64,
        destination: &Path,
    ) -> Result<(Response, ResumeState), Error> {
        let mut headers = HeaderMap::new();
        self.resume.apply_range(&mut headers, &state)?;

        let request = self.http.get(url).headers(headers);
        let response = self.http.send(request).await?;

        if !response.status().is_success() {
            let error = map_download_error(&response, destination);

            if matches!(error, Error::RangeNotSatisfiable { .. }) {
                self.resume.reset(&state).await?;
            }

            return Err(error);
        }

        match self.resume.validate_response(
            &state,
            response.status(),
            response.headers(),
            expected_size,
        ) {
            Ok(validated) => Ok((response, validated)),
            Err(err) => {
                if matches!(err, Error::InvalidContentRange { .. }) {
                    self.resume.reset(&state).await?;
                }
                Err(err)
            },
        }
    }

    /// Streams the response body to the partial file.
    ///
    /// This reads chunks from the response stream and writes them
    /// incrementally to disk, emitting `Progress` events after each chunk.
    ///
    /// # Cancellation
    /// The cancellation token is checked before each chunk write, allowing
    /// the operation to abort promptly when cancelled.
    ///
    /// # Performance
    /// Using `bytes_stream()` avoids buffering the entire response in
    /// memory, keeping memory usage proportional to the chunk size.
    async fn download_body(
        &self,
        job_id: u64,
        response: Response,
        state: &ResumeState,
        expected_size: u64,
        cancel: &Cancel,
    ) -> Result<(), Error> {
        let mut file = self.resume.open(state).await?;

        let mut written = state.offset();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            cancel.check()?;

            let bytes = chunk.map_err(HttpError::StreamInterrupted)?;

            file.write_all(&bytes).await.map_err(|e| io_error(&state.part_path, e))?;

            if self.progress.is_active() {
                written += bytes.len() as u64;
                self.progress.emit(ProgressEvent::Progress {
                    job_id,
                    bytes_written: written,
                    total_size: expected_size,
                });
            }
        }

        file.flush().await.map_err(|e| io_error(&state.part_path, e))?;

        Ok(())
    }

    /// Finalizes an existing partial file that was already complete.
    ///
    /// This verifies the checksum (if required) and renames the `.part` file
    /// to its final name. Unlike `verify_and_finalize`, this does not check
    /// the size again because the file was previously determined to be
    /// complete.
    ///
    /// # Windows compatibility
    /// On Windows, `rename` cannot overwrite an existing file, so the
    /// destination is removed first if it exists.
    async fn finalize_existing_part(
        &mut self,
        state: &ResumeState,
        destination: &Path,
        checksum: &ChecksumSpec,
    ) -> Result<(), Error> {
        if self.verify_mode == VerificationMode::SizeAndChecksum
            && !self
                .verifier
                .verify(&state.part_path, checksum)
                .await
                .map_err(|e| io_error(&state.part_path, e))?
        {
            self.resume.reset(state).await?;

            return Err(Error::ChecksumMismatch { path: state.part_path.clone() });
        }

        if cfg!(windows) && try_exists(destination).await.map_err(|e| io_error(destination, e))? {
            remove_file(destination).await.map_err(|e| io_error(destination, e))?;
        }
        rename(&state.part_path, destination)
            .await
            .map_err(|e| io_error(destination, e))?;

        Ok(())
    }

    /// Verifies the downloaded file and renames it to its final destination.
    ///
    /// The verification process:
    /// 1. Check that the file size matches `expected_size`.
    /// 2. If verification mode is `SizeAndChecksum`, compute and compare the file's checksum
    ///    against the expected value.
    ///
    /// If either check fails, the partial file is removed and an error is
    /// returned.
    ///
    /// # Returns
    /// `Outcome::Resumed` if the file was resumed from a partial download,
    /// `Outcome::Downloaded` if it was downloaded from scratch.
    async fn verify_and_finalize(
        &mut self,
        destination: &Path,
        state: &ResumeState,
        expected_size: u64,
        checksum: &ChecksumSpec,
    ) -> Result<Outcome, Error> {
        let actual_size = self.resume.current_size(state).await?;

        if actual_size != expected_size {
            return Err(Error::SizeMismatch {
                path: state.part_path.clone(),
                expected: expected_size,
                actual: actual_size,
            });
        }

        if self.verify_mode == VerificationMode::SizeAndChecksum
            && !self
                .verifier
                .verify(&state.part_path, checksum)
                .await
                .map_err(|e| io_error(&state.part_path, e))?
        {
            self.resume.reset(state).await?;
            return Err(Error::ChecksumMismatch { path: state.part_path.clone() });
        }

        if cfg!(windows) && try_exists(destination).await.map_err(|e| io_error(destination, e))? {
            remove_file(destination).await.map_err(|e| io_error(destination, e))?;
        }
        rename(&state.part_path, destination)
            .await
            .map_err(|e| io_error(destination, e))?;

        if state.is_resuming() {
            Ok(Outcome::Resumed)
        } else {
            Ok(Outcome::Downloaded)
        }
    }
}
