use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use async_channel::{Sender, bounded, unbounded};
use futures_util::Stream;
use tokio::spawn;

use crate::{
    Error,
    api::ResourceClient,
    cancel::Cancel,
    download::progress::{ProgressEmitter, ProgressEvent},
    model::{Item, PublicKey},
};

/// A page listing task for a parallel directory walk.
///
/// Represents a request to list one page of a directory at a specific offset.
struct PageTask {
    path: String,
    offset: usize,
}

/// Walks a directory tree in parallel using multiple API workers.
///
/// `ParallelWalker` provides high-performance recursive directory traversal
/// by:
/// 1. Using a work queue of page requests (`PageTask`).
/// 2. Spawning multiple workers to fetch pages concurrently.
/// 3. Enqueuing subdirectory pages as they are discovered.
/// 4. Streaming items to the consumer as they are fetched.
///
/// # Performance
/// The parallel walker can significantly outperform the sequential walker
/// on deep directory trees or when the API has high latency, because
/// multiple page requests are in flight simultaneously.
///
/// # Memory
/// The walker uses a bounded channel for items to prevent unbounded buffering
/// if the consumer is slower than the walker.
pub(crate) struct ParallelWalker {
    api: ResourceClient,
    public_key: PublicKey,
    cancel: Cancel,
    max_listing_workers: usize,
    page_size: usize,
    item_buffer: usize,
    progress: ProgressEmitter,
}

impl ParallelWalker {
    /// Creates a new parallel walker with default settings.
    ///
    /// Defaults:
    /// - Page size: 1000 items
    /// - Item buffer: 2000 items
    ///
    /// The `max_listing_workers` controls how many concurrent API requests
    /// are made for directory listings.
    pub(crate) fn new(
        api: ResourceClient,
        public_key: &PublicKey,
        max_listing_workers: usize,
        cancel: Cancel,
        progress: ProgressEmitter,
    ) -> Self {
        Self {
            api,
            public_key: public_key.clone(),
            cancel,
            max_listing_workers,
            page_size: 1000,
            item_buffer: 2000,
            progress,
        }
    }

    /// Sets the maximum number of concurrent listing workers.
    pub(crate) fn max_listing_workers(mut self, n: usize) -> Self {
        self.max_listing_workers = n.max(1);
        self
    }

    /// Converts the walker into a `Stream` of items.
    ///
    /// The stream yields `Item`s from the directory tree in no particular
    /// order (directory listing order is preserved per page, but pages from
    /// different directories may interleave).
    ///
    /// # Progress events
    /// The walker emits:
    /// - `DiscoveryStarted` when the walk begins.
    /// - `DiscoveryProgress` periodically as items are scanned.
    /// - `DiscoveryFinished` when the walk completes.
    ///
    /// # Cancellation
    /// The stream checks the cancellation token periodically and exits early
    /// if cancelled. The `DiscoveryFinished` event may not be emitted if the
    /// walk is cancelled.
    pub(crate) fn into_stream(self, root_path: String) -> impl Stream<Item = Result<Item, Error>> {
        let (item_tx, item_rx) = bounded::<Result<Item, Error>>(self.item_buffer);
        let (task_tx, task_rx) = unbounded::<PageTask>();
        let pending = Arc::new(AtomicUsize::new(1));
        let _ = task_tx.try_send(PageTask { path: root_path, offset: 0 });

        self.progress.emit(ProgressEvent::DiscoveryStarted);

        let scanned = Arc::new(AtomicU64::new(0));
        let discovered_files = Arc::new(AtomicU64::new(0));
        let discovered_bytes = Arc::new(AtomicU64::new(0));

        for _ in 0..self.max_listing_workers {
            let api = self.api.clone();
            let pk = self.public_key.clone();
            let cancel = self.cancel.clone();
            let task_tx = task_tx.clone();
            let task_rx = task_rx.clone();
            let item_tx = item_tx.clone();
            let pending = pending.clone();
            let page_size = self.page_size;
            let progress = self.progress.clone();
            let scanned = scanned.clone();
            let discovered_files = discovered_files.clone();
            let discovered_bytes = discovered_bytes.clone();

            spawn(async move {
                while let Ok(task) = task_rx.recv().await {
                    if cancel.check().is_err() {
                        finish(
                            &pending,
                            &task_tx,
                            &item_tx,
                            &progress,
                            &discovered_files,
                            &discovered_bytes,
                        );
                        break;
                    }

                    match api.list_page(&pk, &task.path, page_size, task.offset, &cancel).await {
                        Ok(page) => {
                            let Some(embedded) = page.embedded else {
                                finish(
                                    &pending,
                                    &task_tx,
                                    &item_tx,
                                    &progress,
                                    &discovered_files,
                                    &discovered_bytes,
                                );
                                continue;
                            };

                            // If this is the first page and we know the total size,
                            // enqueue the remaining pages in advance.
                            if task.offset == 0
                                && let Some(total) = embedded.total
                            {
                                let mut offset = page_size;
                                while offset < total as usize {
                                    pending.fetch_add(1, Ordering::SeqCst);
                                    if task_tx
                                        .send(PageTask { path: task.path.clone(), offset })
                                        .await
                                        .is_err()
                                    {
                                        pending.fetch_sub(1, Ordering::SeqCst);
                                        break;
                                    }
                                    offset += page_size;
                                }
                            }

                            let items = embedded.items.unwrap_or_default();
                            let got = items.len();

                            // If we got a full page but no total, the directory is paginated
                            // but the API didn't give us a total. Continue fetching.
                            let unknown_total_continue =
                                embedded.total.is_none() && got == page_size;
                            if unknown_total_continue {
                                dispatch(
                                    &pending,
                                    &task_tx,
                                    PageTask {
                                        path: task.path.clone(),
                                        offset: task.offset + got,
                                    },
                                )
                                .await;
                            }

                            let batch_scanned =
                                scanned.fetch_add(got as u64, Ordering::Relaxed) + got as u64;
                            if progress.is_active() {
                                progress.emit(ProgressEvent::DiscoveryProgress {
                                    scanned: batch_scanned,
                                });
                            }

                            for item in items {
                                if item.is_dir()
                                    && let Some(path) = item.path.clone()
                                {
                                    dispatch(&pending, &task_tx, PageTask { path, offset: 0 })
                                        .await;
                                }
                                if item.is_file() {
                                    discovered_files.fetch_add(1, Ordering::Relaxed);
                                    discovered_bytes
                                        .fetch_add(item.size.unwrap_or(0), Ordering::Relaxed);
                                }
                                if item_tx.send(Ok(item)).await.is_err() {
                                    break;
                                }
                            }
                        },
                        Err(err) => {
                            let _ = item_tx.send(Err(err)).await;
                        },
                    }

                    finish(
                        &pending,
                        &task_tx,
                        &item_tx,
                        &progress,
                        &discovered_files,
                        &discovered_bytes,
                    );
                }
            });
        }

        drop(task_tx);
        drop(item_tx);

        item_rx
    }
}

/// Finishes the walker when all tasks are complete.
///
/// This is called after each task completes. When the last pending task
/// finishes, it closes the task and item channels and emits the
/// `DiscoveryFinished` event.
#[allow(clippy::too_many_arguments)]
fn finish(
    pending: &Arc<AtomicUsize>,
    task_tx: &Sender<PageTask>,
    item_tx: &Sender<Result<Item, Error>>,
    progress: &ProgressEmitter,
    discovered_files: &Arc<AtomicU64>,
    discovered_bytes: &Arc<AtomicU64>,
) {
    if pending.fetch_sub(1, Ordering::SeqCst) == 1 {
        task_tx.close();
        item_tx.close();

        if progress.is_active() {
            progress.emit(ProgressEvent::DiscoveryFinished {
                total_files: discovered_files.load(Ordering::Relaxed),
                total_bytes: discovered_bytes.load(Ordering::Relaxed),
            });
        }
    }
}

/// Dispatches a new page task to the queue.
///
/// Increments the pending count, then sends the task. If the send fails
/// (because the receiver is closed), decrements the pending count to avoid
/// a mismatch.
async fn dispatch(pending: &Arc<AtomicUsize>, task_tx: &Sender<PageTask>, task: PageTask) {
    pending.fetch_add(1, Ordering::SeqCst);
    if task_tx.send(task).await.is_err() {
        pending.fetch_sub(1, Ordering::SeqCst);
    }
}
