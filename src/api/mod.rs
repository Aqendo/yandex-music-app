//! Pure-Rust HTTP client for the Yandex Music API.
//!
//! Models the transport and endpoints of the TypeScript reference client
//! (`@dvxch/yandex-music`) so the two stay contract-compatible. Response payloads
//! arrive wrapped in a `{ "result": ... }` envelope, which is unwrapped here; the
//! OAuth endpoints (`oauth.yandex.ru`) return bare snake_case objects instead.
pub mod account;
pub mod auth;
pub mod covers;
pub mod likes;
pub mod models;
pub mod playlists;
pub mod radio;
pub mod search;
pub mod tracks;

use std::sync::{Arc, Mutex};

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_LANGUAGE};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;

pub use models::*;

/// API origin (matches `DEFAULT_BASE_URL` in the reference client).
pub const BASE_URL: &str = "https://api.music.yandex.net";
/// OAuth origin.
pub const OAUTH_BASE_URL: &str = "https://oauth.yandex.ru";
/// Public OAuth client credentials of the official Android app (per reference).
pub const CLIENT_ID: &str = "23cabbbdc6cd418abb4b39c32c41195d";
pub const CLIENT_SECRET: &str = "53bc75238f0c4d08a118e51fe9203300";
/// `X-Yandex-Music-Client` header value (matches the reference client).
pub const YM_CLIENT_HEADER: &str = "YandexMusicAndroid/24023621";
pub const USER_AGENT: &str = "yandex-music-native/0.1";

/// A Yandex OAuth token pair.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
}

/// Typed error for the whole API layer.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not authenticated: {0}")]
    Unauthorized(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("api error: {message} (HTTP {status})")]
    Status { status: u16, message: String },
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("auth error: {0}")]
    Auth(String),
}

impl ApiError {
    /// Whether the underlying problem is a stale/expired token.
    pub fn is_auth(&self) -> bool {
        matches!(
            self,
            ApiError::Unauthorized(_) | ApiError::Forbidden(_)
        )
    }
}

/// Build an `X-Yandex-Music-Client`/`Accept-Language` header map.
fn base_headers(language: &str) -> Result<HeaderMap, ApiError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Yandex-Music-Client",
        HeaderValue::from_static(YM_CLIENT_HEADER),
    );
    if !language.is_empty() {
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_str(language).unwrap());
    }
    Ok(headers)
}

/// Extract a human-readable message from an error response body.
fn extract_error_message(bytes: &[u8]) -> String {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return String::from_utf8_lossy(bytes).trim().to_string();
    };
    let pick = |obj: &serde_json::Value| -> Option<String> {
        let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() && message.is_empty() {
            None
        } else {
            let mut parts = Vec::new();
            if !name.is_empty() {
                parts.push(name);
            }
            if !message.is_empty() {
                parts.push(message);
            }
            Some(parts.join(": "))
        }
    };
    // Priority: nested `result` error -> top-level `{name,message}` -> `error` object.
    let found = value
        .get("result")
        .and_then(pick)
        .or_else(|| pick(&value))
        .or_else(|| value.get("error").and_then(pick));
    if let Some(found) = found {
        return found;
    }
    let error = value.get("error").and_then(|v| v.as_str()).unwrap_or("");
    let description = value
        .get("errorDescription")
        .or_else(|| value.get("error_description"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    [error, description]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn map_status(status: StatusCode, message: String) -> ApiError {
    match status.as_u16() {
        401 => ApiError::Unauthorized(message),
        403 => ApiError::Forbidden(message),
        404 => ApiError::NotFound(message),
        400 => ApiError::BadRequest(message),
        code => ApiError::Status {
            status: code,
            message,
        },
    }
}

/// The Yandex Music API client.
///
/// Cheap to clone; all mutable state (token, resolved uid) is shared via
/// `Arc<Mutex<..>>`, so a single client can be handed to concurrent workers.
#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    tokens: Arc<Mutex<Option<AuthTokens>>>,
    uid: Arc<Mutex<Option<i64>>>,
    language: String,
}

impl ApiClient {
    /// Create a client for the given response language.
    pub fn new(language: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("failed to build HTTP client");
        Self {
            http,
            tokens: Arc::new(Mutex::new(None)),
            uid: Arc::new(Mutex::new(None)),
            language: language.into(),
        }
    }

    /// Attach a saved token pair.
    pub fn set_tokens(&self, tokens: Option<AuthTokens>) {
        *self.tokens.lock().unwrap() = tokens;
    }

    /// Current token pair, if any.
    pub fn tokens(&self) -> Option<AuthTokens> {
        self.tokens.lock().unwrap().clone()
    }

    /// Remember the authenticated account uid (needed for `/users/{uid}/...`).
    pub fn set_uid(&self, uid: Option<i64>) {
        *self.uid.lock().unwrap() = uid;
    }

    /// The authenticated account uid, if known.
    pub fn uid(&self) -> Option<i64> {
        *self.uid.lock().unwrap()
    }

    /// Build the per-request headers, attaching the OAuth token when present.
    fn headers(&self) -> Result<HeaderMap, ApiError> {
        let mut headers = base_headers(&self.language)?;
        if let Some(tokens) = self.tokens.lock().unwrap().as_ref() {
            if !tokens.access_token.is_empty() {
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("OAuth {}", tokens.access_token)).unwrap(),
                );
            }
        }
        Ok(headers)
    }

    /// Perform a `GET` and unwrap the `{ result }` envelope.
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ApiError> {
        let url = format!("{BASE_URL}{path}");
        let mut request = self.http.get(&url).headers(self.headers()?);
        if !query.is_empty() {
            request = request.query(query);
        }
        self.send(request).await
    }

    /// Perform a form-encoded `POST` and unwrap the `{ result }` envelope.
    pub async fn post_form<T: DeserializeOwned>(
        &self,
        path: &str,
        form: &[(&str, String)],
    ) -> Result<T, ApiError> {
        let url = format!("{BASE_URL}{path}");
        let request = self.http.post(&url).headers(self.headers()?).form(form);
        self.send(request).await
    }

    /// Perform a JSON `POST` and unwrap the `{ result }` envelope.
    pub async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, ApiError> {
        let url = format!("{BASE_URL}{path}");
        let request = self.http.post(&url).headers(self.headers()?).json(body);
        self.send(request).await
    }

    /// Raw `GET` (no envelope parsing) returning the response bytes.
    ///
    /// Used for the download-info XML and later for streaming audio. Accepts an
    /// absolute URL or a path relative to the API origin.
    pub async fn raw_get(&self, url: &str) -> Result<Vec<u8>, ApiError> {
        let url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("{BASE_URL}{url}")
        };
        let response = self.http.get(&url).headers(self.headers()?).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let message = extract_error_message(&bytes);
            let message = if message.is_empty() {
                format!("HTTP {}", status.as_u16())
            } else {
                message
            };
            return Err(map_status(status, message));
        }
        Ok(bytes.to_vec())
    }

    /// Send a request and parse the response, unwrapping the envelope.
    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ApiError> {
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let message = extract_error_message(&bytes);
            let message = if message.is_empty() {
                format!("HTTP {}", status.as_u16())
            } else {
                message
            };
            return Err(map_status(status, message));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::BadResponse(format!("invalid JSON: {e}")))?;
        let result = value
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        serde_json::from_value(result).map_err(|e| ApiError::BadResponse(e.to_string()))
    }
}

/// The batch "track-ids" form helper: repeats the key per id, as the reference
/// client does.
pub fn ids_form<'a>(key: &'a str, ids: &[Id]) -> Vec<(&'a str, String)> {
    ids.iter()
        .map(|id| (key, id.0.clone()))
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_unwraps_result() {
        let value: serde_json::Value =
            serde_json::from_str(r#"{"invocationInfo": {}, "result": {"uid": 1}}"#).unwrap();
        let result = value.get("result").cloned().unwrap();
        let parsed: Account = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.uid, Some(1));
    }

    #[test]
    fn ids_form_repeats_key() {
        let form = ids_form("track-ids", &[Id("1".into()), Id("2".into())]);
        assert_eq!(
            form,
            vec![("track-ids", "1".to_string()), ("track-ids", "2".to_string())]
        );
    }
}
