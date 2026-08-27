use serde::{Deserialize, Serialize};

use crate::checksum::ChecksumSpec;

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub type_field: Option<String>,
    pub path: Option<String>,
    pub size: Option<u64>,
    pub md5: Option<String>,
    pub sha256: Option<String>,
    pub file: Option<String>,
}

impl Item {
    pub fn is_dir(&self) -> bool {
        self.type_field.as_deref() == Some("dir")
    }

    pub fn is_file(&self) -> bool {
        self.type_field.as_deref() == Some("file")
    }

    pub fn checksum_spec(&self) -> ChecksumSpec {
        ChecksumSpec::from_parts(self.md5.clone(), self.sha256.clone())
    }
}
