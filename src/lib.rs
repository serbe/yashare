mod client;
mod error;
mod transport;

pub use client::DownloadClient;
pub use error::{Error, Result, io_error};

pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

// #[tokio::main]
// async fn main() -> Result<()> {
//     let shutdown = CancellationToken::new();

//     {
//         let shutdown = shutdown.clone();

//         tokio::spawn(async move {
//             let _ = tokio::signal::ctrl_c().await;
//             eprintln!("shutdown requested");
//             shutdown.cancel();
//         });
//     }

//     let downloader = Downloader::new(DownloadConfig::default())?;

//     downloader
//         .download(
//             "https://example.com/archive.tar.gz",
//             "archive.tar.gz",
//             shutdown,
//         )
//         .await?;

//     Ok(())
// }
