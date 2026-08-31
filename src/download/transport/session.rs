use std::path::Path;

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
        model::outcome::Outcome,
        resume::{
            manager::ResumeManager,
            state::{ResumeAction, ResumeState},
        },
    },
    error::HttpError,
    fs::{ChecksumSpec, FileVerifier, VerificationMode},
    io_error,
};

/// Manages the download of a single file, including resuming and verification.
pub(crate) struct DownloadSession<'a> {
    http: &'a HttpClient,
    resume: &'a ResumeManager,
    verifier: &'a mut FileVerifier,
    verify_mode: VerificationMode,
}

impl<'a> DownloadSession<'a> {
    /// Creates a new `DownloadSession` with the given dependencies.
    pub(crate) fn new(
        http: &'a HttpClient,
        resume: &'a ResumeManager,
        verifier: &'a mut FileVerifier,
        verify_mode: VerificationMode,
    ) -> Self {
        Self { http, resume, verifier, verify_mode }
    }

    /// Runs the download session, downloading the file from the given URL to the destination path.
    pub async fn run(
        &mut self,
        url: &str,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
        cancel: &Cancel,
    ) -> Result<Outcome, Error> {
        let mut state = self.resume.inspect(destination, expected_size).await?;

        if matches!(state.action, ResumeAction::Finalize) {
            self.finalize_existing_part(&state, destination, checksum).await?;
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

        let (response, state) = self.send_request(url, state, expected_size, destination).await?;

        self.download_body(response, &state, cancel).await?;

        self.verify_and_finalize(destination, &state, expected_size, checksum).await
    }

    /// Sends an HTTP request to the given URL, with the given headers and expected size.
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

            // Битый/несовместимый .part — если это не сбросить, следующая попытка
            // снова наткнётся на тот же Range-запрос и ту же ошибку.
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
                // Content-Range не совпал с ожидаемым — локальным данным больше нельзя
                // доверять, начинаем с нуля.
                if matches!(err, Error::InvalidContentRange { .. }) {
                    self.resume.reset(&state).await?;
                }
                Err(err)
            },
        }
    }

    /// Downloads the body of the given response to the file specified in the state.
    async fn download_body(
        &self,
        response: Response,
        state: &ResumeState,
        cancel: &Cancel,
    ) -> Result<(), Error> {
        let mut file = self.resume.open(state).await?;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            cancel.check()?;

            let bytes = chunk.map_err(HttpError::StreamInterrupted)?;

            file.write_all(&bytes).await.map_err(|e| io_error(&state.part_path, e))?;
        }

        file.flush().await.map_err(|e| io_error(&state.part_path, e))?;

        Ok(())
    }

    /// Verifies and finalizes the existing part of the file, if one exists.
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

    /// Verifies and finalizes the downloaded file, checking the size and checksum if required.
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
