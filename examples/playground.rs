use std::path::PathBuf;

use tracing::{debug, error, info};
use yashare::{Cancel, Error, PublicKey, YaShareClient};

const KEY: &str = "https://disk.yandex.ru/d/965DOIGYMrcE-w";

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();

    let cancel = Cancel::new();
    cancel.spawn()?;

    let dest_dir = PathBuf::from("./downloads");

    let client = YaShareClient::default();
    let public_key = PublicKey::parse(KEY)?;

    debug!("public_key: {}", public_key.as_api_string());

    let result = client.download_all(&public_key, &dest_dir, &cancel).await?;

    info!("Готово!");
    info!("Загружено: {}", result.stats.downloaded());
    info!("Докачано: {}", result.stats.resumed());
    info!("Пропущено: {}", result.stats.skipped());
    info!("Ошибок: {}", result.stats.failed());

    if !result.failures.is_empty() {
        info!("\nОшибки:");

        for failure in &result.failures {
            error!("{} -> {}", failure.path, failure.error);
        }
    }

    Ok(())
}
