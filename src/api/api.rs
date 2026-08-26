use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use url::Url;

use crate::{
    Error,
    api::{http_client::HttpClient, retry::RetryPolicy},
    model::{Link, Resource, ResourceField, build_fields},
    utils::PublicKey,
};

#[derive(Clone)]
pub(crate) struct ApiClient {
    http: HttpClient,
    retry: RetryPolicy,
    api_base: Url,
    fields: String,
}

impl ApiClient {
    pub(crate) fn new(http: HttpClient, retry: RetryPolicy, api_base: Url) -> Self {
        ApiClient::new_with_fields(
            http,
            retry,
            api_base,
            build_fields(&ResourceField::default()),
        )
    }

    pub fn new_with_fields(
        http: HttpClient,
        retry: RetryPolicy,
        api_base: Url,
        fields: String,
    ) -> Self {
        Self {
            http,
            retry,
            api_base,
            fields,
        }
    }

    pub fn set_fields(&mut self, fields: String) {
        self.fields = fields;
    }

    pub(crate) async fn get_public_resource<T>(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        extra: &[(&str, String)],
        shutdown: &CancellationToken,
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

        self.http
            .get_json(url.as_str(), &self.retry, shutdown)
            .await
    }

    pub(crate) async fn download_href(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        shutdown: &CancellationToken,
    ) -> Result<Link, Error> {
        self.get_public_resource(public_key, path, &[], shutdown)
            .await
    }

    pub(crate) async fn list_page(
        &self,
        public_key: &PublicKey,
        path: &str,
        limit: usize,
        offset: usize,
        shutdown: &CancellationToken,
    ) -> Result<Resource, Error> {
        self.get_public_resource(
            public_key,
            Some(path),
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
            shutdown,
        )
        .await
    }

    pub(crate) async fn get_json<T>(
        &self,
        url: &str,
        shutdown: &CancellationToken,
    ) -> Result<T, Error>
    where
        T: DeserializeOwned,
    {
        self.http.get_json(url, &self.retry, shutdown).await
    }

    pub(crate) async fn resource_meta(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        shutdown: &CancellationToken,
    ) -> Result<Resource, Error> {
        self.get_public_resource(public_key, path, &[], shutdown)
            .await
    }

    fn get_public_api_url(&self) -> Result<Url, Error> {
        let mut url = self.api_base.clone();
        url.path_segments_mut()
            .map_err(|_| Error::InvalidPath(self.api_base.clone().to_string()))?
            .push("public")
            .push("resources");
        Ok(url)
    }
}
