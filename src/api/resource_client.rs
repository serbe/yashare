use std::marker::PhantomData;

use reqwest::{Response, header::HeaderMap};
use serde::de::DeserializeOwned;
use serde_json::from_slice;
use tracing::debug;
use url::Url;

use crate::{
    Error,
    api::{http::HttpClient, map_error_response},
    cancel::Cancel,
    error::{ApiError, ClientError},
    model::{Link, PublicKey, Resource, ResourceField, build_fields},
    retry::{Attempt, RetryPolicy, run},
};

/// A client for interacting with the Yandex Share API.
#[derive(Clone)]
pub struct ResourceClient {
    http: HttpClient,
    retry: RetryPolicy,
    api_base: Url,
    fields: String,
}

impl ResourceClient {
    /// Creates a new [`ResourceClient`] with the given [`HttpClient`], [`RetryPolicy`], and API
    /// base URL.
    pub(crate) fn new(http: HttpClient, retry: RetryPolicy, api_base: Url) -> Self {
        ResourceClient::new_with_fields(
            http,
            retry,
            api_base,
            build_fields(&ResourceField::default()),
        )
    }

    /// Creates a new [`ResourceClient`] with the given [`HttpClient`], [`RetryPolicy`], API
    /// base URL, and fields.
    pub fn new_with_fields(
        http: HttpClient,
        retry: RetryPolicy,
        api_base: Url,
        fields: String,
    ) -> Self {
        Self { http, retry, api_base, fields }
    }

    /// Sets the fields to request from the API for resource metadata.
    pub fn set_fields(&mut self, fields: String) {
        self.fields = fields;
    }

    /// Makes a GET request to the API and deserializes the response into a [`DeserializeOwned`]
    /// type.
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

    /// Makes a GET request to the public API and deserializes the response into a
    /// [`DeserializeOwned`] type.
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

    /// Makes a GET request to the public API and deserializes the response into a
    /// [`Link`] type.
    pub(crate) async fn get_download_link(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Link, Error> {
        self.get_public_resource(public_key, path, &[], cancel).await
    }

    /// Makes a GET request to the public API and deserializes the response into a
    /// [`Resource`] type.
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

    /// Makes a GET request to the public API and deserializes the response into a
    /// [`Resource`] type.
    pub(crate) async fn resource_meta(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Resource, Error> {
        self.get_public_resource(public_key, path, &[], cancel).await
    }

    /// Makes a GET request to the public API and deserializes the response into a
    /// [`Url`] type.
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

    /// Sends a request with the given headers and returns the response.
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

    /// Sends a request with the given URL and returns the response.
    pub(crate) async fn send_checked(&self, url: &str) -> Result<Response, Error> {
        self.send_checked_with_headers(url, HeaderMap::new()).await
    }
}

/// A wrapper around `Attempt` that makes a GET request to the public API and deserializes
/// the response into a [`DeserializeOwned`] type.
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
