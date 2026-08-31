use std::collections::VecDeque;

use futures_util::{Stream, stream::try_unfold};

use crate::{
    Error,
    api::resource_client::ResourceClient,
    cancel::Cancel,
    model::{Item, PublicKey},
};

const DEFAULT_PAGE_SIZE: usize = 1000;

#[derive(Debug)]
struct DirectoryState {
    path: String,
    offset: usize,
    total: Option<usize>,
}

pub(crate) struct Walker {
    api: ResourceClient,
    public_key: PublicKey,
    cancel: Cancel,

    pending: Vec<DirectoryState>,
    current: Option<DirectoryState>,

    buffer: VecDeque<Item>,

    page_size: usize,
}

impl Walker {
    pub(crate) fn new(
        api: ResourceClient,
        public_key: &PublicKey,
        root_path: impl Into<String>,
        cancel: Cancel,
    ) -> Self {
        Self {
            api,
            public_key: public_key.clone(),
            cancel,

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
            self.cancel.check()?;

            if let Some(item) = self.buffer.pop_front() {
                self.handle_item(&item);
                return Ok(Some(item));
            }

            if let Some(current) = &self.current
                && let Some(total) = current.total
                && current.offset >= total
            {
                self.current = None;
                continue;
            }

            if self.current.is_none() {
                self.current = self.pending.pop();
            }

            let Some(current) = self.current.as_mut() else {
                return Ok(None);
            };

            let page = self
                .api
                .list_page(
                    &self.public_key,
                    &current.path,
                    self.page_size,
                    current.offset,
                    &self.cancel,
                )
                .await?;

            let Some(embedded) = page.embedded else {
                self.current = None;
                continue;
            };

            if let Some(total) = embedded.total {
                current.total = Some(total as usize);
            }

            let items = embedded.items.unwrap_or_default();

            if items.is_empty() {
                self.current = None;
                continue;
            }

            current.offset += items.len();

            self.buffer.extend(items);
        }
    }

    fn handle_item(&mut self, item: &Item) {
        if !item.is_dir() {
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
