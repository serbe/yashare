use std::time::Duration;

use bytes::Bytes;
use reqwest::{Client, RequestBuilder, Response, header::RETRY_AFTER};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

use crate::{
    Error,
    api::retry::{RetryDecision, RetryPolicy},
    model::ApiErrorResponse,
    utils::sleep_or_cancel,
};

#[derive(Clone)]
pub(crate) struct HttpClient {
    client: Client,
}

impl HttpClient {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    pub(crate) fn get(&self, url: &str) -> RequestBuilder {
        self.client.get(url)
    }

    pub(crate) async fn send_checked(
        &self,
        make_request: impl Fn() -> RequestBuilder,
    ) -> Result<Response, Error> {
        let response = make_request().send().await.map_err(Error::Connect)?;

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();

        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs);

        Err(match response.json::<ApiErrorResponse>().await {
            Ok(api_error) => match api_error.error.as_str() {
                "DiskNotFoundError" => Error::NotFound(api_error.description),
                _ => Error::Api {
                    status,
                    error: api_error.error,
                    message: api_error.message,
                    description: api_error.description,
                    retry_after,
                },
            },
            Err(_) => Error::Status {
                status,
                retry_after,
            },
        })
    }

    pub(crate) async fn get_json<T>(
        &self,
        url: &str,
        retry: &RetryPolicy,
        shutdown: &CancellationToken,
    ) -> Result<T, Error>
    where
        T: DeserializeOwned,
    {
        let mut last_error = None;

        for attempt in 1..=retry.max_attempts {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let response = match self.send_checked(|| self.client.get(url)).await {
                Ok(response) => response,
                Err(error) => {
                    if self
                        .should_retry(error, retry, attempt, shutdown, &mut last_error)
                        .await?
                    {
                        continue;
                    }
                    break;
                }
            };

            let bytes = match self.read_body(response, shutdown).await {
                Ok(bytes) => bytes,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(error) => {
                    if self
                        .should_retry(error, retry, attempt, shutdown, &mut last_error)
                        .await?
                    {
                        continue;
                    }
                    break;
                }
            };

            return serde_json::from_slice::<T>(&bytes).map_err(Error::Json);
        }

        Err(last_error.unwrap_or(Error::Cancelled))
    }

    async fn should_retry(
        &self,
        error: Error,
        retry: &RetryPolicy,
        attempt: usize,
        shutdown: &CancellationToken,
        last_error: &mut Option<Error>,
    ) -> Result<bool, Error> {
        match retry.decide(&error, attempt) {
            RetryDecision::Abort => Err(error),
            RetryDecision::RetryAfter(delay) => {
                *last_error = Some(error);

                if attempt >= retry.max_attempts {
                    return Ok(false);
                }
                if sleep_or_cancel(delay, shutdown).await {
                    return Err(Error::Cancelled);
                }
                Ok(true)
            }
        }
    }

    async fn read_body(
        &self,
        response: Response,
        shutdown: &CancellationToken,
    ) -> Result<Bytes, Error> {
        tokio::select! {
            result = response.bytes() => result.map_err(Error::BodyInterrupted),
            _ = shutdown.cancelled() => Err(Error::Cancelled),
        }
    }
}
