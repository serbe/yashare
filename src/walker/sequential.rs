use std::collections::VecDeque;

use futures_util::{Stream, stream::try_unfold};

use crate::{
    Error,
    api::ResourceClient,
    cancel::Cancel,
    model::{Item, PublicKey},
};

/// Default number of items to request per API page.
const DEFAULT_PAGE_SIZE: usize = 1000;

/// State for a directory being walked.
#[derive(Debug)]
struct DirectoryState {
    path: String,
    offset: usize,
    total: Option<usize>,
}

/// Sequentially walks a directory tree using a depth-first traversal.
///
/// `Walker` is a simpler alternative to `ParallelWalker` that walks the tree
/// sequentially, one directory at a time. It uses less memory and fewer
/// concurrent connections, but may be slower for large directories.
///
/// # Traversal order
/// The walker uses a depth-first traversal: it walks the current directory
/// to completion before moving to the next. Subdirectories are enqueued and
/// processed in the order they are discovered.
///
/// # Memory
/// The walker buffers a single page of items at a time. Subdirectory paths
/// are stored in a queue for later traversal.
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
    /// Creates a new sequential walker.
    ///
    /// The walker starts at `root_path` and will traverse all subdirectories.
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

    /// Sets the number of items to fetch per API page.
    pub(crate) fn page_size(mut self, page_size: usize) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    /// Fetches the next item from the walker.
    ///
    /// This may make API requests to fetch additional pages or traverse new
    /// directories. The walker maintains internal state to resume where it
    /// left off.
    ///
    /// # Cancellation
    /// Checks the cancellation token before each API request and returns
    /// `Error::Cancelled` if the token is cancelled.
    async fn next_item(&mut self) -> Result<Option<Item>, Error> {
        loop {
            self.cancel.check()?;

            // Return buffered items first
            if let Some(item) = self.buffer.pop_front() {
                self.handle_item(&item);
                return Ok(Some(item));
            }

            // If we've reached the end of the current directory, move to the next
            if let Some(current) = &self.current
                && let Some(total) = current.total
                && current.offset >= total
            {
                self.current = None;
                continue;
            }

            // If no current directory, pop the next from the queue
            if self.current.is_none() {
                self.current = self.pending.pop();
            }

            let Some(current) = self.current.as_mut() else {
                return Ok(None);
            };

            // Fetch the next page of the current directory
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

    /// Processes an item after it's yielded to the consumer.
    ///
    /// If the item is a directory, it's enqueued for later traversal.
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

    /// Converts the walker into a `Stream` of items.
    pub(crate) fn into_stream(self) -> impl Stream<Item = Result<Item, Error>> {
        try_unfold(self, |mut walker| async move {
            match walker.next_item().await? {
                Some(item) => Ok(Some((item, walker))),
                None => Ok(None),
            }
        })
    }
}
