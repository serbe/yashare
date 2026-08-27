use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Link {
    pub method: String,
    pub href: String,
    pub templated: bool,
}
