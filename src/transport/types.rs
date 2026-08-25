use jiff::Timestamp;
use serde::{Deserialize, Serialize};

const DEFAULT_PAGE_SIZE: usize = 1000;

#[derive(Debug, Clone)]
pub enum ChecksumSpec {
    Md5(String),
    Sha256(String),
    Both { md5: String, sha256: String },
    SizeOnly,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Resource {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub type_field: Option<String>,
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

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub item_type: Option<String>,
    pub path: Option<String>,
    pub size: Option<u64>,
    pub md5: Option<String>,
    pub sha256: Option<String>,
    pub file: Option<String>,
}

impl Item {
    pub fn checksum_spec(&self) -> ChecksumSpec {
        match (&self.md5, &self.sha256) {
            (Some(md5), Some(sha256)) => ChecksumSpec::Both {
                md5: md5.clone(),
                sha256: sha256.clone(),
            },
            (Some(md5), None) => ChecksumSpec::Md5(md5.clone()),
            (None, Some(sha256)) => ChecksumSpec::Sha256(sha256.clone()),
            (None, None) => ChecksumSpec::SizeOnly,
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    pub is_owned: Option<bool>,
    pub is_root: Option<bool>,
    pub rights: Option<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct CommentIds {
    pub public_resource: Option<String>,
    pub private_resource: Option<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Exif {
    pub date_time: Option<String>,
    pub gps_latitude: Option<String>,
    pub gps_longitude: Option<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PhotoSize {
    pub url: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Link {
    pub method: String,
    pub href: String,
    pub templated: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseError {
    pub message: String,
    pub description: String,
    pub error: String,
}

#[cfg(test)]
mod tests {
    use crate::transport::{
        CommentIdsField, EmbeddedField, ExifField, FieldPath, ItemField, PhotoSizeField,
        ResourceField, ShareField, build_fields,
    };

    #[test]
    fn render_root_field() {
        let field = ResourceField::Name;

        assert_eq!(field.to_path(), "name");
    }

    #[test]
    fn render_embedded_field() {
        let field = ResourceField::Embedded(EmbeddedField::PublicKey);

        assert_eq!(field.to_path(), "_embedded.public_key");
    }

    #[test]
    fn render_embedded_item_field() {
        let field = ResourceField::Embedded(EmbeddedField::Items(ItemField::Name));

        assert_eq!(field.to_path(), "_embedded.items.name");
    }

    #[test]
    fn render_share_field() {
        let field = ResourceField::Share(ShareField::Rights);

        assert_eq!(field.to_path(), "share.rights");
    }

    #[test]
    fn render_comment_ids_field() {
        let field = ResourceField::CommentIds(CommentIdsField::PublicResource);

        assert_eq!(field.to_path(), "comment_ids.public_resource");
    }

    #[test]
    fn render_exif_field() {
        let field = ResourceField::Exif(ExifField::GpsLatitude);

        assert_eq!(field.to_path(), "exif.gps_latitude");
    }

    #[test]
    fn render_photo_size_field() {
        let field = ResourceField::Sizes(PhotoSizeField::Url);

        assert_eq!(field.to_path(), "sizes.url");
    }

    #[test]
    fn build_multiple_fields() {
        let fields = build_fields(&[
            ResourceField::Name,
            ResourceField::Path,
            ResourceField::Embedded(EmbeddedField::Items(ItemField::Name)),
            ResourceField::Embedded(EmbeddedField::Items(ItemField::Size)),
        ]);

        assert_eq!(
            fields,
            "name,path,_embedded.items.name,_embedded.items.size"
        );
    }

    #[test]
    fn build_single_field() {
        let fields = build_fields(&[ResourceField::Name]);

        assert_eq!(fields, "name");
    }

    #[test]
    fn build_empty_field_list() {
        let fields = build_fields(&[]);

        assert_eq!(fields, "");
    }

    #[test]
    fn all_fields_contains_expected_paths() {
        let fields = build_fields(&ResourceField::all());

        assert!(fields.contains("name"));
        assert!(fields.contains("path"));
        assert!(fields.contains("public_key"));
        assert!(fields.contains("share.rights"));
        assert!(fields.contains("comment_ids.public_resource"));
        assert!(fields.contains("exif.date_time"));
        assert!(fields.contains("sizes.url"));
        assert!(fields.contains("_embedded.total"));
        assert!(fields.contains("_embedded.items.name"));
        assert!(fields.contains("_embedded.items.sha256"));
    }

    #[test]
    fn all_fields_do_not_contain_duplicates() {
        use std::collections::HashSet;

        let fields = ResourceField::all();

        let rendered: Vec<String> = fields.iter().map(FieldPath::to_path).collect();

        let unique: HashSet<_> = rendered.iter().collect();

        assert_eq!(rendered.len(), unique.len(), "duplicate fields found");
    }

    #[test]
    fn build_default_fields() {
        use std::collections::HashSet;

        let fields = build_fields(&ResourceField::default());
        let actual: HashSet<&str> = fields.split(',').collect();

        let expected: HashSet<&str> = [
            "name",
            "type",
            "path",
            "size",
            "md5",
            "sha256",
            "file",
            "_embedded.total",
            "_embedded.items.name",
            "_embedded.items.type",
            "_embedded.items.path",
            "_embedded.items.size",
            "_embedded.items.md5",
            "_embedded.items.sha256",
            "_embedded.items.file",
        ]
        .iter()
        .cloned()
        .collect();

        assert_eq!(
            actual, expected,
            "Default fields should contain all expected fields"
        );
    }
}
