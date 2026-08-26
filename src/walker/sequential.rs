use std::collections::VecDeque;

use futures_util::{Stream, stream::try_unfold};
use tokio_util::sync::CancellationToken;

use crate::{Error, api::api::ApiClient, model::Item, utils::PublicKey};

const DEFAULT_PAGE_SIZE: usize = 1000;

#[derive(Debug)]
struct DirectoryState {
    path: String,
    offset: usize,
    total: Option<usize>,
}

pub(crate) struct Walker {
    api: ApiClient,
    public_key: PublicKey,
    shutdown: CancellationToken,

    pending: Vec<DirectoryState>,
    current: Option<DirectoryState>,

    buffer: VecDeque<Item>,

    page_size: usize,
}

impl Walker {
    pub(crate) fn new(
        api: ApiClient,
        public_key: &PublicKey,
        root_path: impl Into<String>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            api,
            public_key: public_key.clone(),
            shutdown,

            pending: vec![DirectoryState {
                path: root_path.into(),
                offset: 0,
                total: None,
            }],

            current: None,
            buffer: VecDeque::new(),
            page_size: DEFAULT_PAGE_SIZE,
        }
    }

    pub(crate) fn page_size(mut self, page_size: usize) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    async fn next_item(&mut self) -> Result<Option<Item>, Error> {
        loop {
            if self.shutdown.is_cancelled() {
                return Err(Error::Cancelled);
            }

            /*
             * Сначала отдаём уже загруженные элементы.
             */
            if let Some(item) = self.buffer.pop_front() {
                self.handle_item(&item);
                return Ok(Some(item));
            }

            /*
             * Если текущий каталог закончился,
             * переходим к следующему.
             */
            if let Some(current) = &self.current
                && let Some(total) = current.total
                && current.offset >= total
            {
                self.current = None;
                continue;
            }

            /*
             * Если текущего каталога нет,
             * берём следующий из DFS-стека.
             */
            if self.current.is_none() {
                self.current = self.pending.pop();
            }

            let Some(current) = self.current.as_mut() else {
                return Ok(None);
            };

            /*
             * Запрашиваем следующую страницу.
             */
            let page = self
                .api
                .list_page(
                    &self.public_key,
                    &current.path,
                    self.page_size,
                    current.offset,
                    &self.shutdown,
                )
                .await?;

            let Some(embedded) = page.embedded else {
                self.current = None;
                continue;
            };

            /*
             * Сохраняем total только если API его вернул.
             */
            if let Some(total) = embedded.total {
                current.total = Some(total as usize);
            }

            let items = embedded.items.unwrap_or_default();

            /*
             * Пустая страница означает конец каталога,
             * даже если total отсутствует.
             */
            if items.is_empty() {
                self.current = None;
                continue;
            }

            current.offset += items.len();

            self.buffer.extend(items);
        }
    }

    fn handle_item(&mut self, item: &Item) {
        if item.item_type.as_deref() != Some("dir") {
            return;
        }

        let Some(path) = item.path.as_deref() else {
            return;
        };

        self.pending.push(DirectoryState {
            path: path.to_owned(),
            offset: 0,
            total: None,
        });
    }

    pub(crate) fn into_stream(self) -> impl Stream<Item = Result<Item, Error>> {
        try_unfold(self, |mut walker| async move {
            match walker.next_item().await? {
                Some(item) => Ok(Some((item, walker))),
                None => Ok(None),
            }
        })
    }
}
