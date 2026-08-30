use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_channel::{bounded, unbounded};
use futures_util::Stream;

use crate::{
    Error,
    api::ResourceClient,
    cancel::Cancel,
    model::{Item, PublicKey},
};

struct PageTask {
    path: String,
    offset: usize,
}

pub(crate) struct ParallelWalker {
    api: ResourceClient,
    public_key: PublicKey,
    cancel: Cancel,
    max_listing_workers: usize,
    page_size: usize,
    item_buffer: usize,
}

impl ParallelWalker {
    pub(crate) fn new(
        api: ResourceClient,
        public_key: &PublicKey,
        max_listing_workers: usize,
        cancel: Cancel,
    ) -> Self {
        Self {
            api,
            public_key: public_key.clone(),
            cancel,
            max_listing_workers,
            page_size: 1000,
            item_buffer: 2000,
        }
    }

    pub(crate) fn max_listing_workers(mut self, n: usize) -> Self {
        self.max_listing_workers = n.max(1);
        self
    }

    pub(crate) fn into_stream(self, root_path: String) -> impl Stream<Item = Result<Item, Error>> {
        let (item_tx, item_rx) = bounded::<Result<Item, Error>>(self.item_buffer);
        let (task_tx, task_rx) = unbounded::<PageTask>();
        let pending = Arc::new(AtomicUsize::new(1));
        let _ = task_tx.try_send(PageTask { path: root_path, offset: 0 });

        for _ in 0..self.max_listing_workers {
            let api = self.api.clone();
            let pk = self.public_key.clone();
            let cancel = self.cancel.clone();
            let task_tx = task_tx.clone();
            let task_rx = task_rx.clone();
            let item_tx = item_tx.clone();
            let pending = pending.clone();
            let page_size = self.page_size;

            tokio::spawn(async move {
                while let Ok(task) = task_rx.recv().await {
                    if cancel.check().is_err() {
                        finish(&pending, &task_tx, &item_tx);
                        break;
                    }

                    match api.list_page(&pk, &task.path, page_size, task.offset, &cancel).await {
                        Ok(page) => {
                            let Some(embedded) = page.embedded else {
                                finish(&pending, &task_tx, &item_tx);
                                continue;
                            };

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

                            for item in items {
                                if item.is_dir()
                                    && let Some(path) = item.path.clone()
                                {
                                    dispatch(&pending, &task_tx, PageTask { path, offset: 0 })
                                        .await;
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

                    finish(&pending, &task_tx, &item_tx);
                }
            });
        }

        drop(task_tx);
        drop(item_tx);

        item_rx
    }
}

fn finish(
    pending: &Arc<AtomicUsize>,
    task_tx: &async_channel::Sender<PageTask>,
    item_tx: &async_channel::Sender<Result<Item, Error>>,
) {
    if pending.fetch_sub(1, Ordering::SeqCst) == 1 {
        task_tx.close();
        item_tx.close();
    }
}

async fn dispatch(
    pending: &Arc<AtomicUsize>,
    task_tx: &async_channel::Sender<PageTask>,
    task: PageTask,
) {
    pending.fetch_add(1, Ordering::SeqCst);
    if task_tx.send(task).await.is_err() {
        pending.fetch_sub(1, Ordering::SeqCst);
    }
}
