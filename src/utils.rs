use std::time::Duration;

use tokio_util::sync::CancellationToken;

pub(crate) async fn sleep_or_cancel(duration: Duration, token: &CancellationToken) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = token.cancelled() => true,
    }
}
