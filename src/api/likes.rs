//! Like/dislike endpoints.
use serde::Deserialize;

use crate::api::models::{Id, TrackShort};
use crate::api::{ids_form, ApiClient, ApiError};

/// The liked-tracks library payload (mirrors the reference `TracksList`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracksLibrary {
    #[serde(default)]
    pub tracks: Vec<TrackShort>,
    #[serde(default)]
    pub revision: Option<i64>,
    #[serde(default)]
    pub uid: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct LikesResponse {
    #[serde(default)]
    library: Option<TracksLibrary>,
}

fn required_uid(client: &ApiClient) -> Result<i64, ApiError> {
    client
        .uid()
        .ok_or_else(|| ApiError::Unauthorized("account uid is unknown; call init first".into()))
}

impl ApiClient {
    /// Fetch the liked tracks (references only).
    pub async fn users_likes_tracks(&self) -> Result<Vec<TrackShort>, ApiError> {
        let uid = required_uid(self)?;
        let response: LikesResponse = self.get(&format!("/users/{uid}/likes/tracks"), &[]).await?;
        Ok(response.library.unwrap_or_default().tracks)
    }

    /// Add likes to one or many tracks.
    pub async fn likes_tracks_add(&self, ids: &[Id]) -> Result<(), ApiError> {
        let uid = required_uid(self)?;
        self.post_form::<serde_json::Value>(
            &format!("/users/{uid}/likes/tracks/add-multiple"),
            &ids_form("track-ids", ids),
        )
        .await?;
        Ok(())
    }

    /// Remove likes from one or many tracks.
    pub async fn likes_tracks_remove(&self, ids: &[Id]) -> Result<(), ApiError> {
        let uid = required_uid(self)?;
        self.post_form::<serde_json::Value>(
            &format!("/users/{uid}/likes/tracks/remove"),
            &ids_form("track-ids", ids),
        )
        .await?;
        Ok(())
    }

    /// Add dislikes to one or many tracks.
    pub async fn dislikes_tracks_add(&self, ids: &[Id]) -> Result<(), ApiError> {
        let uid = required_uid(self)?;
        self.post_form::<serde_json::Value>(
            &format!("/users/{uid}/dislikes/tracks/add-multiple"),
            &ids_form("track-ids", ids),
        )
        .await?;
        Ok(())
    }

    /// Remove dislikes from one or many tracks.
    pub async fn dislikes_tracks_remove(&self, ids: &[Id]) -> Result<(), ApiError> {
        let uid = required_uid(self)?;
        self.post_form::<serde_json::Value>(
            &format!("/users/{uid}/dislikes/tracks/remove"),
            &ids_form("track-ids", ids),
        )
        .await?;
        Ok(())
    }

    /// Hydrate a list of track references into full tracks.
    pub async fn hydrate_tracks(
        &self,
        refs: &[TrackShort],
    ) -> Result<Vec<crate::api::Track>, ApiError> {
        let ids: Vec<Id> = refs.iter().filter_map(|t| t.id.clone()).collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.get_list("track", &ids, &[("with-positions", "true".to_string())])
            .await
    }
}
