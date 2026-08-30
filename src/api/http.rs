use bytes::Bytes;
use reqwest::{Client, RequestBuilder, Response, header::HeaderMap};

use crate::{Error, api::map_error_response, cancel::Cancel, error::HttpError};

/// Чистый транспорт: отправить запрос, прочитать тело. Ничего не знает
/// про retry-политику, cancellation-семантику решений или формат ошибок
/// конкретного API — этим занимаются `retry::run` и `api::error_mapping`.
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

    pub(crate) async fn send(&self, request: RequestBuilder) -> Result<Response, Error> {
        request.send().await.map_err(|e| Error::Http(HttpError::Request(e)))
    }

    /// Чтение тела — единственное место, где транспорту всё же нужна
    /// отмена (иначе зависшее чтение нельзя прервать).
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

    pub(crate) async fn send_checked_with_headers(
        &self,
        url: &str,
        headers: HeaderMap,
    ) -> Result<Response, Error> {
        let request = self.get(url).headers(headers);
        let response = self.send(request).await?;

        if response.status().is_success() {
            Ok(response)
        } else {
            Err(Error::Api(map_error_response(response).await))
        }
    }

    pub(crate) async fn send_checked(&self, url: &str) -> Result<Response, Error> {
        self.send_checked_with_headers(url, HeaderMap::new()).await
    }
}
