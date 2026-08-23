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
