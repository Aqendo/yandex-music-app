//! Track endpoints: batch fetch and download-info resolution.
use md5::{Digest, Md5};
use quick_xml::events::Event;
use serde::Deserialize;

use crate::api::models::{Id, Track};
use crate::api::{ids_form, ApiClient, ApiError};

/// Salt used to sign direct download links (from the reference client).
const SIGN_SALT: &str = "XGRlBW9FXlekgbPrRHuSiA";

/// One downloadable variant of a track.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub bitrate_in_kbps: Option<i64>,
    #[serde(default)]
    pub preview: Option<bool>,
    #[serde(default)]
    pub download_info_url: Option<String>,
    #[serde(default)]
    pub direct: Option<bool>,
}

impl ApiClient {
    /// Fetch one or many tracks by id (mirrors `client.tracks`).
    pub async fn tracks(&self, ids: &[Id]) -> Result<Vec<Track>, ApiError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut form = ids_form("track-ids", ids);
        form.push(("with-positions", "true".to_string()));
        self.post_form("/tracks", &form).await
    }

    /// Fetch the available download variants for a track.
    pub async fn tracks_download_info(&self, track_id: &Id) -> Result<Vec<DownloadInfo>, ApiError> {
        self.get(&format!("/tracks/{track_id}/download-info"), &[]).await
    }
}

/// A resolved, short-lived direct download link.
pub struct DirectLink {
    pub url: String,
}

/// Parse the download-info XML (`<download-info><host/><path/><ts/><s/></download-info>`).
fn parse_download_info_xml(xml: &str) -> Result<(String, String, String, String), ApiError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut current = String::new();
    let mut host = String::new();
    let mut path = String::new();
    let mut ts = String::new();
    let mut s = String::new();
    let mut in_download_info = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = e.name();
                if name.as_ref() == b"download-info" {
                    in_download_info = true;
                } else if in_download_info {
                    current = name.as_ref().iter().map(|b| *b as char).collect();
                }
            }
            Ok(Event::Text(ref e)) => {
                if !current.is_empty() {
                    let text = e.unescape().unwrap_or_default().into_owned();
                    match current.as_str() {
                        "host" => host = text,
                        "path" => path = text,
                        "ts" => ts = text,
                        "s" => s = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name().as_ref() == b"download-info" {
                    break;
                }
                current.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(ApiError::BadResponse(format!("invalid XML: {e}"))),
            _ => {}
        }
    }
    if host.is_empty() || path.is_empty() {
        return Err(ApiError::BadResponse("download-info XML missing host/path".into()));
    }
    Ok((host, path, ts, s))
}

/// Build the signed direct mp3 URL from the parsed download-info fields.
///
/// `sign = md5(SALT + path[1..] + s)`; the final URL is
/// `https://<host>/get-mp3/<sign>/<ts><path>` (matches the reference client).
pub fn build_direct_link(host: &str, path: &str, ts: &str, s: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(SIGN_SALT.as_bytes());
    hasher.update(&path.as_bytes()[1..]);
    hasher.update(s.as_bytes());
    let sign = hasher.finalize();
    let sign = format!("{sign:x}");
    format!("https://{host}/get-mp3/{sign}/{ts}{path}")
}

impl ApiClient {
    /// Resolve a direct, short-lived streaming URL for a track.
    ///
    /// Chooses the highest-bitrate non-preview lossy variant (mp3/aac), matching
    /// the app's "lossy mp3 streaming" target.
    pub async fn resolve_direct_link(&self, track_id: &Id) -> Result<String, ApiError> {
        let variants = self.tracks_download_info(track_id).await?;
        let best = variants
            .iter()
            .filter(|v| v.preview != Some(true))
            .filter(|v| v.codec.as_deref() == Some("mp3") || v.codec.as_deref() == Some("aac"))
            .max_by_key(|v| v.bitrate_in_kbps.unwrap_or(0))
            .ok_or_else(|| ApiError::BadResponse("no lossy download variant".into()))?;
        let info_url = best
            .download_info_url
            .as_deref()
            .ok_or_else(|| ApiError::BadResponse("download variant has no URL".into()))?;
        let xml = String::from_utf8_lossy(&self.raw_get(info_url).await?).into_owned();
        let (host, path, ts, s) = parse_download_info_xml(&xml)?;
        Ok(build_direct_link(&host, &path, &ts, &s))
    }

    /// Resolve a direct stream URL for a full `Track` model.
    pub async fn resolve_track_stream(&self, track: &Track) -> Result<String, ApiError> {
        let id = track
            .id
            .as_ref()
            .ok_or_else(|| ApiError::BadRequest("track has no id".into()))?;
        self.resolve_direct_link(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"<download-info>
        <host>ve-cdntest-music-1.s3.yandex.net</host>
        <path>/get-mp3/07b2ebd58cafc55c5b1b2a29413f2e1d/0e7e280c06f4512c57d4d40b50e9b8c1c68c0c30/1</path>
        <ts>0e7e280c06</ts>
        <s>37a173b50c</s>
        <codec>mp3</codec>
        <bitrateInKbps>192</bitrateInKbps>
    </download-info>"#;

    #[test]
    fn parses_download_info_xml() {
        let (host, path, ts, s) = parse_download_info_xml(SAMPLE_XML).unwrap();
        assert!(host.contains("s3.yandex.net"));
        assert!(path.starts_with("/get-mp3/"));
        assert_eq!(ts, "0e7e280c06");
        assert_eq!(s, "37a173b50c");
    }

    #[test]
    fn builds_signed_link() {
        let (host, path, ts, s) = parse_download_info_xml(SAMPLE_XML).unwrap();
        let url = build_direct_link(&host, &path, &ts, &s);
        assert!(url.starts_with("https://"));
        assert!(url.contains("/get-mp3/"));
    }

    #[test]
    fn picks_best_lossy_variant() {
        let variants = [
            DownloadInfo {
                codec: Some("mp3".into()),
                bitrate_in_kbps: Some(128),
                preview: Some(true),
                download_info_url: None,
                direct: None,
            },
            DownloadInfo {
                codec: Some("mp3".into()),
                bitrate_in_kbps: Some(320),
                preview: Some(false),
                download_info_url: Some("x".into()),
                direct: None,
            },
        ];
        let best = variants
            .iter()
            .filter(|v| v.preview != Some(true))
            .max_by_key(|v| v.bitrate_in_kbps.unwrap_or(0))
            .unwrap();
        assert_eq!(best.bitrate_in_kbps, Some(320));
    }
}
