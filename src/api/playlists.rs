//! Playlist endpoints.
use crate::api::{ApiClient, ApiError, Playlist};

fn required_uid(client: &ApiClient) -> Result<i64, ApiError> {
    client
        .uid()
        .ok_or_else(|| ApiError::Unauthorized("account uid is unknown; call init first".into()))
}

impl ApiClient {
    /// List the user's playlists (summaries with covers and track counts).
    pub async fn users_playlists_list(&self) -> Result<Vec<Playlist>, ApiError> {
        let uid = required_uid(self)?;
        self.get(&format!("/users/{uid}/playlists/list"), &[]).await
    }

    /// Fetch a single playlist by kind (includes its tracks).
    pub async fn users_playlist(&self, kind: i64) -> Result<Playlist, ApiError> {
        let uid = required_uid(self)?;
        self.get(&format!("/users/{uid}/playlists/{kind}"), &[])
            .await
    }
}
