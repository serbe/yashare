use serde::{Deserialize, Serialize};

use crate::{fs::ChecksumSpec, model::ResourceType};

/// Represents a single file or directory item from the API response.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub type_field: Option<ResourceType>,
    pub path: Option<String>,
    pub size: Option<u64>,
    pub md5: Option<String>,
    pub sha256: Option<String>,
    pub file: Option<String>,
}

impl Item {
    /// Returns `true` if the item is a directory.
    pub fn is_dir(&self) -> bool {
        self.type_field == Some(ResourceType::Dir)
    }

    /// Returns `true` if the item is a file.
    pub fn is_file(&self) -> bool {
        self.type_field == Some(ResourceType::File)
    }

    /// Returns the checksum specification for the item.
    pub fn checksum_spec(&self) -> ChecksumSpec {
        ChecksumSpec::from_parts(self.md5.clone(), self.sha256.clone())
    }
}
