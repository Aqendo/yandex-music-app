//! Rotor (personal radio / "My Wave") endpoints.
use crate::api::{ApiClient, ApiError, StationResult, StationTracksResult};

/// The personal infinite station ("My Wave").
pub const MY_WAVE: &str = "user:onyourwave";

/// Feedback kind sent to the rotor while listening.
#[derive(Clone, Copy, Debug)]
pub enum Feedback {
    RadioStarted,
    TrackStarted,
    TrackFinished,
    Skip,
}

impl Feedback {
    fn as_str(self) -> &'static str {
        match self {
            Feedback::RadioStarted => "radioStarted",
            Feedback::TrackStarted => "trackStarted",
            Feedback::TrackFinished => "trackFinished",
            Feedback::Skip => "skip",
        }
    }
}

/// Mood/energy preset accepted by the station settings endpoint.
pub const MOODS: [&str; 5] = ["fun", "active", "calm", "sad", "all"];
/// Diversity preset.
pub const DIVERSITIES: [&str; 4] = ["favorite", "popular", "discover", "default"];
/// Language preset.
pub const LANGUAGES: [&str; 3] = ["not-russian", "russian", "any"];

impl ApiClient {
    /// Fetch info about a station, including its personalization settings.
    pub async fn rotor_station_info(&self, station: &str) -> Result<Vec<StationResult>, ApiError> {
        self.get(&format!("/rotor/station/{station}/info"), &[])
            .await
    }

    /// Pull the next batch of tracks for a station.
    ///
    /// `queue` is the previous batch id; pass it to keep the wave continuous.
    /// Like the reference client, `settings2` is only sent on the initial batch.
    pub async fn rotor_station_tracks(
        &self,
        station: &str,
        queue: Option<String>,
    ) -> Result<StationTracksResult, ApiError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        match queue {
            Some(queue) => query.push(("queue", queue)),
            None => query.push(("settings2", "true".to_string())),
        }
        self.get(&format!("/rotor/station/{station}/tracks"), &query)
            .await
    }

    /// Send listening feedback for a station.
    pub async fn rotor_station_feedback(
        &self,
        station: &str,
        feedback: Feedback,
        batch_id: Option<&str>,
        track_id: Option<&crate::api::Id>,
        total_played_seconds: Option<i64>,
        from: Option<&str>,
    ) -> Result<(), ApiError> {
        let path = match batch_id {
            Some(batch) => format!("/rotor/station/{station}/feedback?batch-id={batch}"),
            None => format!("/rotor/station/{station}/feedback"),
        };
        let timestamp = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()) as i64;
        let mut form: Vec<(&str, String)> = vec![
            ("type", feedback.as_str().to_string()),
            ("timestamp", timestamp.to_string()),
        ];
        if let Some(track_id) = track_id {
            form.push(("trackId", track_id.0.clone()));
        }
        if let Some(seconds) = total_played_seconds {
            form.push(("totalPlayedSeconds", seconds.to_string()));
        }
        if let Some(from) = from {
            form.push(("from", from.to_string()));
        }
        let _: serde_json::Value = self.post_form(&path, &form).await?;
        Ok(())
    }

    /// Change the personalization settings of a station.
    ///
    /// `mood_energy` is one of [`MOODS`], `diversity` one of [`DIVERSITIES`],
    /// `language` one of [`LANGUAGES`]. Hits the `/settings3` endpoint like the
    /// reference client's `rotorStationSettings2`.
    pub async fn rotor_station_settings(
        &self,
        station: &str,
        mood_energy: &str,
        diversity: &str,
        language: &str,
    ) -> Result<(), ApiError> {
        let body = serde_json::json!({
            "moodEnergy": mood_energy,
            "diversity": diversity,
            "language": language,
            "type": "rotor",
        });
        let _: serde_json::Value = self
            .post_json(&format!("/rotor/station/{station}/settings3"), &body)
            .await?;
        Ok(())
    }
}
