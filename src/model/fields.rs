use crate::field_enum;

field_enum! {
    pub enum ItemField {
        leaf {
            Name => "name",
            Type => "type",
            Path => "path",
            Size => "size",
            Md5 => "md5",
            Sha256 => "sha256",
            File => "file",
        }
        nested {}
    }
}

field_enum! {
    pub enum ShareField {
        leaf {
            IsOwned => "is_owned",
            IsRoot => "is_root",
            Rights => "rights",
        }
        nested {}
    }
}

field_enum! {
    pub enum CommentIdsField {
        leaf {
            PublicResource => "public_resource",
            PrivateResource => "private_resource",
        }
        nested {}
    }
}

field_enum! {
    pub enum ExifField {
        leaf {
            DateTime => "date_time",
            GpsLatitude => "gps_latitude",
            GpsLongitude => "gps_longitude",
        }
        nested {}
    }
}

field_enum! {
    pub enum PhotoSizeField {
        leaf {
            Url => "url",
            Name => "name",
        }
        nested {}
    }
}

field_enum! {
    pub enum EmbeddedField {
        leaf {
            Total => "total",
            Limit => "limit",
            Offset => "offset",
            Path => "path",
            Sort => "sort",
            PublicKey => "public_key",
        }
        nested {
            Items(ItemField) => "items",
        }
    }
}

field_enum! {
    pub enum ResourceField {
        leaf {
            Name => "name",
            Type => "type",
            Path => "path",
            Created => "created",
            Modified => "modified",
            Size => "size",
            MimeType => "mime_type",
            Md5 => "md5",
            Sha256 => "sha256",
            Preview => "preview",
            PublicKey => "public_key",
            PublicUrl => "public_url",
            MediaType => "media_type",
            File => "file",
            ResourceId => "resource_id",
            Revision => "revision",
            PhotosliceTime => "photoslice_time",
            ViewsCount => "views_count",
            AntivirusStatus => "antivirus_status",
        }
        nested {
            Embedded(EmbeddedField) => "_embedded",
            Share(ShareField) => "share",
            CommentIds(CommentIdsField) => "comment_ids",
            Exif(ExifField) => "exif",
            Sizes(PhotoSizeField) => "sizes",
        }
    }
}

pub trait FieldPath {
    fn write(&self, out: &mut String);

    fn to_path(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

pub fn build_fields(fields: &[ResourceField]) -> String {
    fields
        .iter()
        .map(FieldPath::to_path)
        .collect::<Vec<_>>()
        .join(",")
}

impl ResourceField {
    pub fn default() -> Vec<Self> {
        vec![
            Self::Name,
            Self::Type,
            Self::Path,
            Self::Size,
            Self::Md5,
            Self::Sha256,
            Self::File,
            Self::Embedded(EmbeddedField::Total),
            Self::Embedded(EmbeddedField::Items(ItemField::Name)),
            Self::Embedded(EmbeddedField::Items(ItemField::Type)),
            Self::Embedded(EmbeddedField::Items(ItemField::Path)),
            Self::Embedded(EmbeddedField::Items(ItemField::Size)),
            Self::Embedded(EmbeddedField::Items(ItemField::Md5)),
            Self::Embedded(EmbeddedField::Items(ItemField::Sha256)),
            Self::Embedded(EmbeddedField::Items(ItemField::File)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
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
