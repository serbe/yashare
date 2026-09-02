use bytes::Bytes;
use reqwest::{Client, RequestBuilder, Response};

use crate::{Error, cancel::Cancel, error::HttpError};

/// Thin wrapper around [`reqwest::Client`] used throughout the crate.
///
/// `HttpClient` centralizes HTTP-related behavior:
/// - Error conversion from `reqwest` errors to crate errors.
/// - Cancellation-aware body reading.
/// - Consistent client configuration across the application.
///
/// # Clone behavior
/// `HttpClient` implements `Clone` because `reqwest::Client` is cheap to
/// clone (it shares an inner connection pool). Cloning creates a new
/// reference to the same connection pool, not a new pool.
#[derive(Clone)]
pub(crate) struct HttpClient {
    client: Client,
}

impl HttpClient {
    /// Creates a new HTTP client wrapper from a `reqwest::Client`.
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// Creates a `GET` request builder for the given URL.
    ///
    /// This delegates to `reqwest::Client::get` and returns the builder
    /// for further customization (headers, query parameters, etc.).
    pub(crate) fn get(&self, url: &str) -> RequestBuilder {
        self.client.get(url)
    }

    /// Executes the request and converts transport-level failures into the
    /// crate's error type.
    ///
    /// # Error mapping
    /// All `reqwest::Error`s from `send()` are wrapped in
    /// `Error::Http(HttpError::Request(e))`. This includes connection
    /// failures, timeouts, and TLS errors.
    ///
    /// # Note
    /// This does NOT check the response status code. Callers are responsible
    /// for checking status codes and converting non-success responses to
    /// appropriate errors using `map_error_response` or `map_download_error`.
    pub(crate) async fn send(&self, request: RequestBuilder) -> Result<Response, Error> {
        request.send().await.map_err(|e| Error::Http(HttpError::Request(e)))
    }

    /// Reads the entire response body.
    ///
    /// The operation is cancellation-aware and aborts immediately when the
    /// associated [`Cancel`] token is triggered. This is used for metadata
    /// responses, which are typically small enough to fit in memory.
    ///
    /// # Cancellation behavior
    /// If the cancellation token is triggered while reading the body, the
    /// read is interrupted and `Error::Cancelled` is returned. The
    /// underlying HTTP connection may be closed, but this is acceptable
    /// because the operation is being cancelled.
    ///
    /// # Errors
    /// - `Error::Cancelled` if the token is triggered.
    /// - `Error::Http(HttpError::BodyInterrupted)` if the body read fails.
    pub(crate) async fn read_body(
        &self,
        response: Response,
        cancel: &Cancel,
    ) -> Result<Bytes, Error> {
        match cancel.race(response.bytes()).await? {
            Ok(bytes) => Ok(bytes),
            Err(err) => Err(Error::Http(HttpError::BodyInterrupted(err))),
        }
    }
}
