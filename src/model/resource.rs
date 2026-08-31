use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::model::Item;

/// Represents the type of a resource (directory or file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Dir,
    File,
}

impl ResourceType {
    /// Returns `true` if the resource type is a directory.
    pub fn is_dir(&self) -> bool {
        matches!(self, Self::Dir)
    }

    /// Returns `true` if the resource type is a file.
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File)
    }
}

/// Full metadata response from Yandex.Disk API.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Resource {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub type_field: Option<ResourceType>,
    pub path: Option<String>,
    #[serde(rename = "_embedded")]
    pub embedded: Option<Embedded>,
    pub created: Option<Timestamp>,
    pub modified: Option<Timestamp>,
    pub size: Option<u64>,
    pub mime_type: Option<String>,
    pub md5: Option<String>,
    pub sha256: Option<String>,
    pub preview: Option<String>,
    pub public_key: Option<String>,
    pub public_url: Option<String>,
    pub media_type: Option<String>,
    pub file: Option<String>,
    pub resource_id: Option<String>,
    pub share: Option<Share>,
    pub revision: Option<i64>,
    pub comment_ids: Option<CommentIds>,
    pub exif: Option<Exif>,
    pub antivirus_status: Option<String>,
    pub photoslice_time: Option<Timestamp>,
    pub sizes: Option<Vec<PhotoSize>>,
    pub views_count: Option<i64>,
}

impl Resource {
    /// Returns `true` if the resource is a directory.
    pub fn is_dir(&self) -> bool {
        self.type_field == Some(ResourceType::Dir)
    }
}

/// Container for paginated directory contents.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Embedded {
    pub total: Option<u64>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub path: Option<String>,
    pub sort: Option<String>,
    pub items: Option<Vec<Item>>,
    pub public_key: Option<String>,
}

/// Represents a shared resource.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    pub is_owned: Option<bool>,
    pub is_root: Option<bool>,
    pub rights: Option<String>,
}

/// Represents the comment IDs for a resource.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct CommentIds {
    pub public_resource: Option<String>,
    pub private_resource: Option<String>,
}

/// Represents the EXIF metadata for a photo.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Exif {
    pub date_time: Option<String>,
    pub gps_latitude: Option<String>,
    pub gps_longitude: Option<String>,
}

/// Represents a photo size.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PhotoSize {
    pub url: Option<String>,
    pub name: Option<String>,
}
