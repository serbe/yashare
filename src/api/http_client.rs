use std::time::Duration;

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

    pub(crate) async fn send_once(
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

    /// Единственное место, где крутится ретрай-цикл с backoff.
    pub(crate) async fn execute<F>(
        &self,
        make_request: F,
        retry: &RetryPolicy,
        shutdown: &CancellationToken,
    ) -> Result<Response, Error>
    where
        F: Fn() -> RequestBuilder,
    {
        let mut last_error = None;

        for attempt in 1..=retry.max_attempts {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            match self.send_once(&make_request).await {
                Ok(response) => return Ok(response),
                Err(error) => match retry.decide(&error, attempt) {
                    RetryDecision::Abort => return Err(error),
                    RetryDecision::RetryAfter(delay) => {
                        last_error = Some(error);
                        if attempt >= retry.max_attempts {
                            break;
                        }
                        if sleep_or_cancel(delay, shutdown).await {
                            return Err(Error::Cancelled);
                        }
                    }
                },
            }
        }

        Err(last_error.unwrap_or(Error::Cancelled))
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

            // одна попытка получить ответ, без вложенного ретрая
            let response = match self.send_once(|| self.client.get(url)).await {
                Ok(response) => response,
                Err(error) => match retry.decide(&error, attempt) {
                    RetryDecision::Abort => return Err(error),
                    RetryDecision::RetryAfter(delay) => {
                        last_error = Some(error);
                        if attempt >= retry.max_attempts {
                            break;
                        }
                        if sleep_or_cancel(delay, shutdown).await {
                            return Err(Error::Cancelled);
                        }
                        continue;
                    }
                },
            };

            let bytes = tokio::select! {
                result = response.bytes() => match result {
                    Ok(bytes) => bytes,
                    Err(source) => {
                        let error = Error::BodyInterrupted(source);
                        match retry.decide(&error, attempt) {
                            RetryDecision::Abort => return Err(error),
                            RetryDecision::RetryAfter(delay) => {
                                last_error = Some(error);
                                if attempt >= retry.max_attempts {
                                    break;
                                }
                                if sleep_or_cancel(delay, shutdown).await {
                                    return Err(Error::Cancelled);
                                }
                                continue;
                            }
                        }
                    }
                },
                _ = shutdown.cancelled() => return Err(Error::Cancelled),
            };

            return serde_json::from_slice::<T>(&bytes).map_err(Error::Json);
        }

        Err(last_error.unwrap_or(Error::Cancelled))
    }
}
