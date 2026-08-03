//! Serde models mirroring the TypeScript reference client's types.
//!
//! The API returns camelCase JSON keys (matching the reference models), so every
//! struct uses `#[serde(rename_all = "camelCase")]`. Fields that the server can
//! emit as `null` are tolerated via null-tolerant deserializers.
use serde::{Deserialize, Deserializer};

/// An id that the API may send as either a JSON string or a number.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Id(pub String);

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) if !s.is_empty() => Ok(Id(s)),
            serde_json::Value::Number(n) => Ok(Id(n.to_string())),
            other => Err(serde::de::Error::custom(format!(
                "expected string or number id, got {other}"
            ))),
        }
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Tolerate `null` for an optional string.
fn de_opt_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

/// Tolerate `null` for a string that we model as non-optional.
fn de_string_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(de_opt_string(deserializer)?.unwrap_or_default())
}

/// Tolerate `null` for a vector field.
fn de_vec_default<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    #[serde(default)]
    pub uid: Option<i64>,
    #[serde(default, deserialize_with = "de_string_default")]
    pub display_name: String,
    #[serde(default, deserialize_with = "de_string_default")]
    pub login: String,
    #[serde(default, deserialize_with = "de_string_default")]
    pub full_name: String,
    #[serde(default)]
    pub region: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub account: Account,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    #[serde(default)]
    pub id: Option<Id>,
    #[serde(default, deserialize_with = "de_string_default")]
    pub name: String,
    #[serde(default)]
    pub cover: Option<Cover>,
    #[serde(default)]
    pub various: Option<bool>,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub genres: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(default)]
    pub id: Option<Id>,
    #[serde(default, deserialize_with = "de_string_default")]
    pub title: String,
    #[serde(default)]
    pub cover_uri: Option<String>,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub track_count: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    #[serde(default)]
    pub id: Option<Id>,
    #[serde(default, deserialize_with = "de_string_default")]
    pub title: String,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub artists: Vec<Artist>,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub cover_uri: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub lyrics_available: Option<bool>,
    #[serde(default)]
    pub explicit: Option<bool>,
    #[serde(default)]
    pub version: Option<String>,
    /// Whether the track is in the user's liked library (from rotor batches).
    #[serde(default)]
    pub liked: Option<bool>,
    /// Whether the track is disliked (from rotor batches).
    #[serde(default)]
    pub disliked: Option<bool>,
}

/// A reference to a track inside a playlist or the liked-tracks library.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackShort {
    #[serde(default)]
    pub id: Option<Id>,
    #[serde(default)]
    pub album_id: Option<Id>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cover {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub items_uri: Vec<String>,
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    #[serde(default)]
    pub uid: Option<i64>,
    #[serde(default)]
    pub kind: Option<i64>,
    #[serde(default, deserialize_with = "de_string_default")]
    pub title: String,
    #[serde(default)]
    pub cover: Option<Cover>,
    #[serde(default)]
    pub track_count: Option<i64>,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub tracks: Vec<TrackShort>,
    #[serde(default)]
    pub playlist_uuid: Option<String>,
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Search {
    #[serde(default)]
    pub best: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "de_string_default")]
    pub text: String,
    #[serde(default)]
    pub albums: Option<SearchBlock<Album>>,
    #[serde(default)]
    pub artists: Option<SearchBlock<Artist>>,
    #[serde(default)]
    pub playlists: Option<SearchBlock<Playlist>>,
    #[serde(default)]
    pub tracks: Option<SearchBlock<Track>>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchBlock<T> {
    pub total: Option<i64>,
    pub results: Vec<T>,
}

impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for SearchBlock<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let total = value.get("total").and_then(|v| v.as_i64());
        let results = match value.get("results") {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .cloned()
                .map(serde_json::from_value::<T>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(serde::de::Error::custom)?,
            Some(_) => return Err(serde::de::Error::custom("results must be an array")),
        };
        Ok(SearchBlock { total, results })
    }
}

// ---------------------------------------------------------------------------
// Rotor ("My Wave") radio
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationId {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Value {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumRestriction {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub possible_values: Vec<Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Restrictions {
    #[serde(default)]
    pub language: Option<EnumRestriction>,
    #[serde(default)]
    pub diversity: Option<EnumRestriction>,
    #[serde(default)]
    pub mood_energy: Option<EnumRestriction>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    #[serde(default)]
    pub id: Option<StationId>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub id_for_from: Option<String>,
    #[serde(default)]
    pub restrictions2: Option<Restrictions>,
    #[serde(default)]
    pub full_image_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationSettings {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub mood_energy: Option<String>,
    #[serde(default)]
    pub diversity: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationResult {
    #[serde(default)]
    pub station: Option<Station>,
    #[serde(default)]
    pub settings2: Option<StationSettings>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    #[serde(default, deserialize_with = "de_vec_default")]
    pub stations: Vec<StationResult>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceItem {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub track: Option<Track>,
    #[serde(default)]
    pub liked: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationTracksResult {
    #[serde(default)]
    pub id: Option<StationId>,
    #[serde(default, deserialize_with = "de_vec_default")]
    pub sequence: Vec<SequenceItem>,
    #[serde(default)]
    pub batch_id: Option<String>,
    #[serde(default)]
    pub radio_session_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_parses_string_and_number() {
        let id: Id = serde_json::from_str(r#""42""#).unwrap();
        assert_eq!(id.0, "42");
        let id: Id = serde_json::from_str(r#"42"#).unwrap();
        assert_eq!(id.0, "42");
    }

    #[test]
    fn track_tolerates_null_vectors() {
        let t: Track = serde_json::from_str(r#"{"id": 1, "title": "T", "artists": null}"#).unwrap();
        assert_eq!(t.artists.len(), 0);
    }

    #[test]
    fn track_parses_camel_case() {
        let t: Track = serde_json::from_str(
            r#"{"id": 1, "title": "T", "coverUri": "x/%%", "durationMs": 2000}"#,
        )
        .unwrap();
        assert_eq!(t.cover_uri.as_deref(), Some("x/%%"));
        assert_eq!(t.duration_ms, Some(2000));
    }

    #[test]
    fn radio_sequence_parses() {
        let r: StationTracksResult = serde_json::from_str(
            r#"{"id": {"type": "user", "tag": "onyourwave"}, "batchId": "b1",
                "sequence": [{"type": "track", "liked": true, "track": {"id": 5, "title": "S"}}]}"#,
        )
        .unwrap();
        assert_eq!(r.batch_id.as_deref(), Some("b1"));
        assert_eq!(r.sequence.len(), 1);
        assert_eq!(r.sequence[0].track.as_ref().unwrap().title, "S");
        assert_eq!(r.sequence[0].liked, Some(true));
    }
}
