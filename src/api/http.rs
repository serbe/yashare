use bytes::Bytes;
use reqwest::{Client, RequestBuilder, Response};

use crate::{Error, cancel::Cancel, error::HttpError};

/// A wrapper around [`reqwest::Client`] that provides HTTP request methods.
#[derive(Clone)]
pub(crate) struct HttpClient {
    client: Client,
}

impl HttpClient {
    /// Creates a new [`HttpClient`] with the given [`reqwest::Client`].
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// Sends a GET request to the given URL.
    pub(crate) fn get(&self, url: &str) -> RequestBuilder {
        self.client.get(url)
    }

    /// Sends a request and returns the response.
    pub(crate) async fn send(&self, request: RequestBuilder) -> Result<Response, Error> {
        request.send().await.map_err(|e| Error::Http(HttpError::Request(e)))
    }

    /// Reads the body of the response, with cancellation support.
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
