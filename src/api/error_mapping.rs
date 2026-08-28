use std::time::Duration;

use reqwest::{Response, header::RETRY_AFTER};

use crate::{Error, model::ApiErrorResponse};

/// Переводит неуспешный HTTP-ответ Yandex.Disk API в доменную `Error`.
/// Это API-специфичное знание (формат тела ошибки, коды типа
/// `DiskNotFoundError`) — оно не должно жить в универсальном HTTP-клиенте.
pub(crate) async fn map_error_response(response: Response) -> Error {
    let status = response.status();
    let retry_after = retry_after(&response);

    match response.json::<ApiErrorResponse>().await {
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
        Err(_) => Error::Status { status, retry_after },
    }
}

fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Отправляет запрос один раз и проверяет статус. Используется и
/// `ResourceClient`, и загрузчиком файлов — оба хотят "послать и получить
/// либо Response, либо смэппленную ошибку", но ни retry, ни cancellation
/// сюда не входят: это забота вызывающего кода через `retry::run`.
pub(crate) async fn send_checked(
    http: &super::HttpClient,
    request: reqwest::RequestBuilder,
) -> Result<Response, Error> {
    let response = http.send(request).await?;

    if response.status().is_success() {
        Ok(response)
    } else {
        Err(map_error_response(response).await)
    }
}
