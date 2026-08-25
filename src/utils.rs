use std::time::Duration;

use tokio_util::sync::CancellationToken;

pub(crate) async fn sleep_or_cancel(duration: Duration, token: &CancellationToken) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = token.cancelled() => true,
    }
}

pub(crate) async fn shutdown_signal() {
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
