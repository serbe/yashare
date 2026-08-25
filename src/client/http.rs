use std::time::Duration;

use reqwest::{Client, header::RETRY_AFTER};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

use crate::{
    Error,
    client::{RetryDecision, RetryPolicy},
    transport::ResponseError,
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

    pub(crate) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.get(url)
    }

    pub(crate) async fn execute<F>(
        &self,
        make_request: F,
        retry: &RetryPolicy,
        shutdown: &CancellationToken,
    ) -> Result<reqwest::Response, Error>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut last_error = None;

        for attempt in 1..=retry.max_attempts {
            if shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let response = match make_request().send().await {
                Ok(response) => response,

                Err(source) => {
                    let error = Error::Connect(source);

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
            };

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

            let error = match response.json::<ResponseError>().await {
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
            };

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
                }
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

            let response = self
                .execute(|| self.client.get(url), retry, shutdown)
                .await?;

            let bytes = tokio::select! {
                result = response.bytes() => {
                    match result {
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
                    }
                }

                _ = shutdown.cancelled() => {
                    return Err(Error::Cancelled);
                }
            };

            return serde_json::from_slice::<T>(&bytes).map_err(Error::Json);
        }

        Err(last_error.unwrap_or(Error::Cancelled))
    }
}
