use serde_json::to_string_pretty;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use yashare::{Error, PublicKey, YaShareClient};

const KEY: &str = "https://disk.yandex.ru/d/965DOIGYMrcE-w";

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "debug".to_string()))
        .init();

    let cancellation_token = CancellationToken::new();

    let token_for_signals = cancellation_token.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("⚠️  Received interrupt signal, shutting down...");
        token_for_signals.cancel();
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

    let value = serde_json::to_value(items).unwrap();
    debug!("{}", to_string_pretty(&value).unwrap());

    Ok(())
}
