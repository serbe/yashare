use std::path::{Path, PathBuf};

use crate::{
    Error, checksum::ChecksumSpec, download::path_safety::safe_relative_path, model::Item,
    public_key::PublicKey,
};

#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub public_key: PublicKey,
    pub item_path: String,
    pub destination: PathBuf,
    pub size: u64,
    pub checksum: ChecksumSpec,
    pub initial_href: Option<String>,
}

impl DownloadJob {
    pub(crate) fn for_download(
        dest_dir: &Path,
        public_key: &PublicKey,
        item: &Item,
    ) -> Result<Self, Error> {
        let item_path = item
            .path
            .as_deref()
            .ok_or_else(|| Error::InvalidPath("item has no path".to_string()))?;
        let destination = dest_dir.join(safe_relative_path(item_path)?);
        Self::from_item(public_key, item, destination)
    }

    pub fn from_item(
        public_key: &PublicKey,
        item: &Item,
        destination: PathBuf,
    ) -> Result<Self, Error> {
        let item_path = item
            .path
            .clone()
            .ok_or_else(|| Error::InvalidPath("item has no path".to_string()))?;

        let size = item
            .size
            .ok_or_else(|| Error::UnexpectedResponse("item has no size".to_string()))?;

        Ok(Self {
            public_key: public_key.clone(),
            item_path,
            destination,
            size,
            checksum: item.checksum_spec(),
            initial_href: item.file.clone(),
        })
    }
}
