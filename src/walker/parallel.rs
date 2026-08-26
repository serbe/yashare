use async_channel::{bounded, unbounded};
use futures_util::Stream;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

use crate::{Error, api::api::ApiClient, model::Item, utils::PublicKey};

struct PageTask {
    path: String,
    offset: usize,
}

pub(crate) struct ParallelWalker {
    api: ApiClient,
    public_key: PublicKey,
    shutdown: CancellationToken,
    listers: usize,
    page_size: usize,
    item_buffer: usize,
}

impl ParallelWalker {
    pub(crate) fn new(
        api: ApiClient,
        public_key: &PublicKey,
        listers: usize,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            api,
            public_key: public_key.clone(),
            shutdown,
            listers,
            page_size: 1000,
            item_buffer: 2000,
        }
    }

    pub(crate) fn listers(mut self, n: usize) -> Self {
        self.listers = n.max(1);
        self
    }

    pub(crate) fn into_stream(self, root_path: String) -> impl Stream<Item = Result<Item, Error>> {
        let (item_tx, item_rx) = bounded::<Result<Item, Error>>(self.item_buffer);
        let (task_tx, task_rx) = unbounded::<PageTask>();

        // счётчик "незавершённых единиц работы" (запрошенных, но ещё не обработанных страниц)
        let pending = Arc::new(AtomicUsize::new(1));
        let _ = task_tx.try_send(PageTask {
            path: root_path,
            offset: 0,
        });

        for _ in 0..self.listers {
            let api = self.api.clone();
            let pk = self.public_key.clone();
            let shutdown = self.shutdown.clone();
            let task_tx = task_tx.clone();
            let task_rx = task_rx.clone();
            let item_tx = item_tx.clone();
            let pending = pending.clone();
            let page_size = self.page_size;

            tokio::spawn(async move {
                while let Ok(task) = task_rx.recv().await {
                    if shutdown.is_cancelled() {
                        finish(&pending, &task_tx, &item_tx);
                        continue;
                    }

                    match api
                        .list_page(&pk, &task.path, page_size, task.offset, &shutdown)
                        .await
                    {
                        Ok(page) => {
                            let Some(embedded) = page.embedded else {
                                finish(&pending, &task_tx, &item_tx);
                                continue;
                            };

                            // если total стал известен именно на этой (первой) странице —
                            // сразу планируем ВСЕ оставшиеся страницы этого каталога параллельно
                            if task.offset == 0 {
                                if let Some(total) = embedded.total {
                                    let mut offset = page_size;
                                    while offset < total as usize {
                                        pending.fetch_add(1, Ordering::SeqCst);
                                        if task_tx
                                            .send(PageTask {
                                                path: task.path.clone(),
                                                offset,
                                            })
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                        offset += page_size;
                                    }
                                }
                            }

                            let items = embedded.items.unwrap_or_default();
                            let got = items.len();

                            // total неизвестен и страница полная — вероятно есть продолжение,
                            // достраиваем цепочку инкрементально (fallback для API без total)
                            let unknown_total_continue =
                                embedded.total.is_none() && got == page_size;
                            if unknown_total_continue {
                                pending.fetch_add(1, Ordering::SeqCst);
                                let _ = task_tx
                                    .send(PageTask {
                                        path: task.path.clone(),
                                        offset: task.offset + got,
                                    })
                                    .await;
                            }

                            for item in items {
                                if item.item_type.as_deref() == Some("dir")
                                    && let Some(path) = item.path.clone()
                                {
                                    pending.fetch_add(1, Ordering::SeqCst);
                                    if task_tx.send(PageTask { path, offset: 0 }).await.is_err() {
                                        pending.fetch_sub(1, Ordering::SeqCst);
                                    }
                                }
                                if item_tx.send(Ok(item)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            let _ = item_tx.send(Err(err)).await;
                        }
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
