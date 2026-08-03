//! Search endpoints.
use crate::api::{ApiClient, ApiError, Search};

impl ApiClient {
    /// Search the catalogue across all entity types.
    pub async fn search(&self, text: &str) -> Result<Search, ApiError> {
        self.get(
            "/search",
            &[
                ("text", text.to_string()),
                ("type", "all".to_string()),
                ("page", "0".to_string()),
                ("nocorrect", "false".to_string()),
                ("playlist-in-best", "true".to_string()),
            ],
        )
        .await
    }
}
