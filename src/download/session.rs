use std::path::Path;

use futures_util::StreamExt;
use reqwest::header::HeaderMap;
use tokio::{fs::rename, io::AsyncWriteExt};

use crate::{
    Error, Outcome, VerificationMode,
    api::HttpClient,
    cancel::Cancel,
    download::{ResumeAction, ResumeManager, ResumeState},
    error::HttpError,
    fs::{ChecksumSpec, FileVerifier},
    io_error,
};

pub(crate) struct DownloadSession<'a> {
    http: &'a HttpClient,
    resume: &'a ResumeManager,
    verifier: &'a mut FileVerifier,
    verify_mode: VerificationMode,
}

impl<'a> DownloadSession<'a> {
    pub(crate) fn new(
        http: &'a HttpClient,
        resume: &'a ResumeManager,
        verifier: &'a mut FileVerifier,
        verify_mode: VerificationMode,
    ) -> Self {
        Self { http, resume, verifier, verify_mode }
    }

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

        if matches!(state.action, ResumeAction::Restart) {
            self.resume.reset(&state).await?;
            state = self.resume.inspect(destination, expected_size).await?;
        }

        let response = self.send_request(url, &state, expected_size).await?;

        state = self.resume.validate_response(
            &state,
            response.status(),
            response.headers(),
            expected_size,
        )?;

        if matches!(state.action, ResumeAction::Restart) {
            self.resume.reset(&state).await?;

            return Err(Error::RangeNotSatisfiable { path: state.part_path.clone() });
        }

        self.download_body(response, &state, cancel).await?;

        self.verify_and_finalize(destination, &state, expected_size, checksum).await
    }

    async fn send_request(
        &self,
        url: &str,
        state: &ResumeState,
        expected_size: u64,
    ) -> Result<reqwest::Response, Error> {
        let mut headers = HeaderMap::new();

        self.resume.apply_range(&mut headers, state)?;

        let response = match self.http.send_checked_with_headers(url, headers).await {
            Ok(response) => response,

            Err(err) if state.is_resuming() && err.is_range_not_satisfiable() => {
                return Err(Error::RangeNotSatisfiable { path: state.part_path.clone() });
            },

            Err(err) => return Err(err),
        };

        let validated = self.resume.validate_response(
            state,
            response.status(),
            response.headers(),
            expected_size,
        )?;

        if matches!(validated.action, ResumeAction::Restart) {
            return Err(Error::RangeNotSatisfiable { path: validated.part_path });
        }

        Ok(response)
    }

    async fn download_body(
        &self,
        response: reqwest::Response,
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

        rename(&state.part_path, destination)
            .await
            .map_err(|e| io_error(destination, e))?;

        Ok(())
    }

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
            return Err(Error::ChecksumMismatch { path: state.part_path.clone() });
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
