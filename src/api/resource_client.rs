use serde::de::DeserializeOwned;
use tracing::debug;
use url::Url;

use crate::{
    Error,
    api::{error_mapping, http::HttpClient},
    cancel::Cancel,
    model::{Link, Resource, ResourceField, build_fields},
    public_key::PublicKey,
    retry::{self, RetryPolicy},
};

#[derive(Clone)]
pub struct ResourceClient {
    http: HttpClient,
    retry: RetryPolicy,
    api_base: Url,
    fields: String,
}

impl ResourceClient {
    pub(crate) fn new(http: HttpClient, retry: RetryPolicy, api_base: Url) -> Self {
        ResourceClient::new_with_fields(
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
        Self { http, retry, api_base, fields }
    }

    pub fn set_fields(&mut self, fields: String) {
        self.fields = fields;
    }

    /// Единственное место, где `ResourceClient` уходит в сеть. Только
    /// `&self`, поэтому `Attempt` здесь можно было бы заменить closure'ом —
    /// но оставляю тот же паттерн, что и в `DownloadWorker`, чтобы оба
    /// места вызова `retry::run` выглядели одинаково.
    async fn get_json<T: DeserializeOwned>(&self, url: &str, cancel: &Cancel) -> Result<T, Error> {
        let policy = self.retry.clone();

        retry::run(
            &policy,
            cancel,
            GetJson {
                resource: self,
                url,
                cancel,
                _marker: std::marker::PhantomData,
            },
        )
        .await
    }

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

    pub(crate) async fn get_download_link(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Link, Error> {
        self.get_public_resource(public_key, path, &[], cancel).await
    }

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

    pub(crate) async fn resource_meta(
        &self,
        public_key: &PublicKey,
        path: Option<&str>,
        cancel: &Cancel,
    ) -> Result<Resource, Error> {
        self.get_public_resource(public_key, path, &[], cancel).await
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

/// Одна попытка `GET + распарсить JSON`. Держит только `&ResourceClient`,
/// поэтому в отличие от `DownloadAttempt` заимствование не мутабельное —
/// но структура та же, что и в download/worker.rs, ради единообразия.
struct GetJson<'a, T> {
    resource: &'a ResourceClient,
    url: &'a str,
    cancel: &'a Cancel,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: DeserializeOwned> retry::Attempt for GetJson<'a, T> {
    type Output = T;

    async fn attempt(&mut self, _attempt_no: usize) -> Result<T, Error> {
        let response =
            error_mapping::send_checked(&self.resource.http, self.resource.http.get(self.url))
                .await?;
        let bytes = self.resource.http.read_body(response, self.cancel).await?;
        serde_json::from_slice(&bytes).map_err(Error::Json)
    }
}
