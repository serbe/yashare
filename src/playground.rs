use tokio_util::sync::CancellationToken;
use tracing::debug;
use yashare::{Error, PublicKey, YaShareClient};

const KEY: &str = "https://disk.yandex.ru/d/965DOIGYMrcE-w";

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "debug".to_string()))
        .init();

    // 1. Создаем токен
    let cancellation_token = CancellationToken::new();

    // 2. Клонируем для обработки сигналов
    let token_for_signals = cancellation_token.clone();

    // 3. Запускаем обработчик сигналов в отдельной задаче
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("⚠️  Received interrupt signal, shutting down...");
        token_for_signals.cancel(); // Сигналим всем задачам о завершении
    });

    let client = YaShareClient::default();

    let public_key = PublicKey::parse(KEY)?;

    debug!("public_key: {}", public_key.as_api_string());

    let meta = client
        .resource_meta(&public_key, None, &cancellation_token)
        .await?;

    let name = meta.name.clone().unwrap();

    debug!("name: {:?}", name);

    let items = meta.embedded.clone().unwrap().items;

    debug!("items: {:?}", items);

    Ok(())
}
