use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiErrorResponse {
    pub message: String,
    pub description: String,
    pub error: String,
}
