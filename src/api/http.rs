use bytes::Bytes;
use reqwest::{Client, RequestBuilder, Response};

use crate::{Error, cancel::Cancel};

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
        request.send().await.map_err(Error::Http)
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
            Err(err) => Err(Error::BodyInterrupted(err)),
        }
    }
}
