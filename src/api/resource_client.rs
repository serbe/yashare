use std::marker::PhantomData;

use reqwest::{Response, header::HeaderMap};
use serde::de::DeserializeOwned;
use serde_json::from_slice;
use tracing::debug;
use url::Url;

use crate::{
    Error,
    api::{error_mapping::map_error_response, http::HttpClient},
    cancel::Cancel,
    error::{ApiError, ClientError},
    model::{Link, PublicKey, Resource, ResourceField, build_fields},
    retry::{Attempt, RetryPolicy, run},
};

/// High-level client for querying metadata from public Yandex.Disk shares.
///
/// `ResourceClient` handles all interactions with the Yandex.Disk public API:
/// - Request construction with query parameters.
/// - Retry logic for transient failures.
/// - Error mapping from HTTP responses to crate errors.
/// - Response deserialization.
///
/// # Field selection
/// The client uses the `fields` query parameter to limit which fields are
/// returned by the API. This reduces response size and improves performance.
/// The field set can be changed dynamically using `set_fields()`.
///
/// # Retry behavior
/// All requests are retried according to the configured `RetryPolicy`.
/// The retry policy handles transient HTTP errors, rate limiting, and
/// service availability issues.
#[derive(Clone)]
pub struct ResourceClient {
    http: HttpClient,
    retry: RetryPolicy,
    api_base: Url,
    fields: String,
}

impl ResourceClient {
    /// Creates a client using the default resource field set.
    ///
    /// The default field set includes: `name`, `type`, `path`, `size`,
    /// `md5`, `sha256`, `file`, and embedded fields for paginated items.
    pub(crate) fn new(http: HttpClient, retry: RetryPolicy, api_base: Url) -> Self {
        ResourceClient::new_with_fields(
            http,
            retry,
            api_base,
            build_fields(&ResourceField::default()),
        )
    }

    /// Creates a client with a custom resource field selection.
    ///
    /// The supplied field list is sent with every metadata request.
    /// This is useful for optimizing responses when only specific fields
    /// are needed.
    fn new_with_fields(
        http: HttpClient,
        retry: RetryPolicy,
        api_base: Url,
        fields: String,
    ) -> Self {
        Self { http, retry, api_base, fields }
    }

    /// Replaces the field selection used for subsequent API requests.
    ///
    /// This is a mutable operation; it affects all future requests made by
    /// this client. For one-off field changes, consider creating a new
    /// client with a custom field set.
    pub(crate) fn set_fields(&mut self, fields: String) {
        self.fields = fields;
    }

    /// Executes a GET request and deserializes the JSON response.
    ///
    /// This is the core request method for the resource client. It handles:
    /// 1. Sending the request via `send_checked()`.
    /// 2. Reading the response body with cancellation support.
    /// 3. Deserializing the body into `T`.
    ///
    /// # Retry
    /// The entire operation is retried according to the client's retry
    /// policy. The `GetJson` wrapper implements `Attempt` to enable
    /// retries at the request level.
    async fn get_json<T: DeserializeOwned>(&self, url: &str, cancel: &Cancel) -> Result<T, Error> {
        let policy = self.retry.clone();

        run(
            &policy,
            cancel,
            GetJson {
                resource: self,
                url,
                cancel,
                _marker: PhantomData,
            },
        )
        .await
    }

    /// Retrieves and deserializes metadata for a public resource.
    ///
    /// This is the most general method for querying public resources. It
    /// constructs the API URL with the appropriate query parameters and
    /// returns the deserialized response.
    ///
    /// # Arguments
    /// - `public_key`: The public key identifying the share.
    /// - `path`: Optional subpath within the share. If `None`, fetches the root of the share.
    /// - `extra`: Additional query parameters to include (e.g., `limit`, `offset` for pagination).
    /// - `cancel`: Cancellation token.
    ///
    /// # Returns
    /// The deserialized response of type `T`. Typically `Resource` for
    /// metadata responses or `Link` for download link responses.
    pub(crate) async fn get_public_resource<T>(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        extra: &[(&str, String)],
        cancel: &Cancel,
    ) -> Result<T, Error>
    where
        T: DeserializeOwned,
    {
        let mut url = self.get_public_api_url()?;

        {
            let mut query = url.query_pairs_mut();

            query.append_pair("public_key", &public_key.as_api_string());

            if let Some(path) = path {
                query.append_pair("path", path);
            }

            for (key, value) in extra {
                query.append_pair(key, value);
            }

            query.append_pair("fields", &self.fields);
        }

        debug!("get_public_resource url: {}", url.as_str());

        self.get_json(url.as_str(), cancel).await
    }

    /// Makes a GET request to the public API and deserializes the response
    /// into a [`Link`] type.
    ///
    /// This is a convenience wrapper around `get_public_resource` for
    /// obtaining download links. The response is expected to be a `Link`
    /// with a direct download URL.
    pub(crate) async fn get_download_link(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Link, Error> {
        self.get_public_resource(public_key, path, &[], cancel).await
    }

    /// Makes a GET request to the public API and deserializes the response
    /// into a [`Resource`] type.
    ///
    /// This fetches a single page of directory contents. The `limit` and
    /// `offset` parameters control pagination.
    pub(crate) async fn list_page(
        &self,
        public_key: &PublicKey,
        path: &str,
        limit: usize,
        offset: usize,
        cancel: &Cancel,
    ) -> Result<Resource, Error> {
        self.get_public_resource(
            public_key,
            Some(path),
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
            cancel,
        )
        .await
    }

    /// Makes a GET request to the public API and deserializes the response
    /// into a [`Resource`] type.
    ///
    /// This fetches metadata for a single resource (file or folder) at the
    /// given path. Use `path = None` to fetch the root of the share.
    pub(crate) async fn resource_meta(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Resource, Error> {
        self.get_public_resource(public_key, path, &[], cancel).await
    }

    /// Constructs the URL for the public API endpoint.
    ///
    /// The URL is `{api_base}/public/resources`. This is the base endpoint
    /// for all public resource queries.
    ///
    /// # Errors
    /// Returns `ClientError::InvalidPath` if the API base URL cannot have
    /// path segments appended.
    fn get_public_api_url(&self) -> Result<Url, Error> {
        let mut url = self.api_base.clone();
        url.path_segments_mut()
            .map_err(|_| {
                Error::Client(ClientError::InvalidPath(self.api_base.clone().to_string()))
            })?
            .push("public")
            .push("resources");
        Ok(url)
    }

    /// Sends a request with custom headers and returns the response.
    ///
    /// This is a low-level method that performs the HTTP request and checks
    /// the response status. Non-success status codes are converted to
    /// `ApiError` using `map_error_response()`.
    ///
    /// # Arguments
    /// - `url`: The request URL.
    /// - `headers`: Custom headers to include in the request.
    ///
    /// # Returns
    /// The `Response` on success (status 2xx). On non-success status,
    /// returns an `Error::Api` with the mapped error.
    pub(crate) async fn send_checked_with_headers(
        &self,
        url: &str,
        headers: HeaderMap,
    ) -> Result<Response, Error> {
        let request = self.http.get(url).headers(headers);
        let response = self.http.send(request).await?;

        if response.status().is_success() {
            Ok(response)
        } else {
            Err(Error::Api(map_error_response(response).await))
        }
    }

    /// Sends a request with no custom headers and returns the response.
    ///
    /// This is a convenience wrapper around `send_checked_with_headers` with
    /// an empty header map.
    pub(crate) async fn send_checked(&self, url: &str) -> Result<Response, Error> {
        self.send_checked_with_headers(url, HeaderMap::new()).await
    }
}

/// A retryable wrapper around a GET request to the public API.
///
/// Implements `Attempt` so that `get_json()` can be retried according to the
/// client's retry policy. Each attempt sends the request and deserializes
/// the response.
struct GetJson<'a, T> {
    resource: &'a ResourceClient,
    url: &'a str,
    cancel: &'a Cancel,
    _marker: PhantomData<T>,
}

impl<'a, T: DeserializeOwned> Attempt for GetJson<'a, T> {
    type Output = T;

    async fn attempt(&mut self, _attempt_no: usize) -> Result<T, Error> {
        let response = self.resource.send_checked(self.url).await?;
        let bytes = self.resource.http.read_body(response, self.cancel).await?;
        from_slice(&bytes).map_err(|e| Error::Api(ApiError::MalformedResponse(e)))
    }
}
