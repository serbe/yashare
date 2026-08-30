use crate::{Error, api::ResourceClient, cancel::Cancel, download::DownloadJob};

pub(crate) struct DownloadLinkProvider {
    api: ResourceClient,
    max_attempts: usize,
}

impl DownloadLinkProvider {
    pub fn new(api: ResourceClient, max_attempts: usize) -> Self {
        Self { api, max_attempts }
    }

    pub async fn get_link(
        &self,
        job: &DownloadJob,
        attempt: usize,
        cancel: &Cancel,
    ) -> Result<String, Error> {
        cancel.check()?;

        let href = if attempt == 1 {
            job.initial_href.clone()
        } else {
            None
        };

        let link = match href {
            Some(href) => href,
            None => {
                self.api
                    .get_download_link(&job.public_key, Some(&job.item_path), cancel)
                    .await?
                    .href
            },
        };

        Ok(link)
    }

    pub fn max_attempts(&self) -> usize {
        self.max_attempts
    }
}
