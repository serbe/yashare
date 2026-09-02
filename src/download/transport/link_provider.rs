use crate::{Error, api::ResourceClient, cancel::Cancel, download::model::DownloadJob};

/// Retrieves download URLs for Yandex.Disk public resources.
///
/// Download links are short-lived (typically a few hours) and may expire
/// during a long-running download. This provider handles obtaining fresh
/// links, either by using a previously-known URL or by requesting a new one
/// from the API.
///
/// # Link freshness
/// The provider does not proactively refresh links; it only requests a new
/// link when the caller explicitly asks for one. The caller is responsible
/// for detecting expired links and requesting a replacement.
pub(crate) struct DownloadLinkProvider {
    api: ResourceClient,
    max_attempts: usize,
}

impl DownloadLinkProvider {
    /// Creates a new link provider with the given API client and maximum
    /// number of link retrieval attempts.
    ///
    /// The `max_attempts` limits how many times the provider will try to
    /// obtain a fresh link before giving up. This is separate from the
    /// download retry policy; it specifically controls link acquisition.
    pub fn new(api: ResourceClient, max_attempts: usize) -> Self {
        Self { api, max_attempts }
    }

    /// Obtains a download URL for the given job, with support for
    /// cancellation.
    ///
    /// # Link reuse
    /// On the first attempt (`attempt == 1`), this uses the `initial_href`
    /// from the job if available — typically a URL obtained during initial
    /// metadata listing. This avoids an extra API call in the common case
    /// where the link is still valid.
    ///
    /// On subsequent attempts, or if no initial URL was provided, this
    /// requests a fresh link from the API using the job's public key and
    /// item path.
    ///
    /// # Cancellation
    /// This method checks the cancellation token before making any API
    /// request. If the token is already cancelled, it returns immediately.
    ///
    /// # Errors
    /// Returns `Error::Api` if the API request fails, or `Error::Cancelled`
    /// if the operation is cancelled.
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

    /// Returns the maximum number of link retrieval attempts.
    ///
    /// This is the total number of times `get_link` may be called before the
    /// provider gives up. It does not include any retries that the caller
    /// may perform on download failures.
    pub fn max_attempts(&self) -> usize {
        self.max_attempts
    }
}
