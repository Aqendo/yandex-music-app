//! OAuth Device Flow for Yandex, mirroring the reference client's `deviceAuth`.
//!
//! Flow: request a device/user code pair -> show the user code + verification URL
//! -> poll until the user confirms -> receive an access token (+ refresh token).
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::api::{ApiClient, ApiError, AuthTokens, OAUTH_BASE_URL};

/// The device/user code pair issued at the start of the flow.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    #[serde(default)]
    pub expires_in: i64,
    #[serde(default)]
    pub interval: i64,
}

/// Whether a poll attempt is "still waiting for the user" vs. a real failure.
#[derive(Clone, Debug)]
pub enum PollOutcome {
    /// The user has not confirmed yet.
    Pending,
    /// A token was issued.
    Token(AuthTokens),
}

impl ApiClient {
    /// Start the device flow: request a user code and return it for display.
    pub async fn request_device_code(
        &self,
        device_id: &str,
        device_name: &str,
    ) -> Result<DeviceCode, ApiError> {
        let form = [
            ("client_id", crate::api::CLIENT_ID.to_string()),
            ("device_id", device_id.to_string()),
            ("device_name", device_name.to_string()),
        ];
        let value: serde_json::Value = self
            .http
            .post(format!("{OAUTH_BASE_URL}/device/code"))
            .form(&form)
            .send()
            .await?
            .json()
            .await?;
        serde_json::from_value(value).map_err(|e| ApiError::Auth(format!("device code: {e}")))
    }

    /// Poll once for the token of a pending device authorization.
    ///
    /// Returns [`PollOutcome::Pending`] while the user has not confirmed.
    pub async fn poll_device_token(&self, device_code: &str) -> Result<PollOutcome, ApiError> {
        let form = [
            ("grant_type", "device_code".to_string()),
            ("code", device_code.to_string()),
            ("client_id", crate::api::CLIENT_ID.to_string()),
            ("client_secret", crate::api::CLIENT_SECRET.to_string()),
        ];
        let response = self
            .http
            .post(format!("{OAUTH_BASE_URL}/token"))
            .form(&form)
            .send()
            .await?;
        let status = response.status();
        let value: serde_json::Value = response.json().await?;
        if status.as_u16() == 400 && value.get("error_description").is_some() {
            // `authorization_pending` and friends mean "keep polling".
            return Ok(PollOutcome::Pending);
        }
        if !status.is_success() {
            let message = value
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or(&status.to_string())
                .to_string();
            return Err(ApiError::Auth(message));
        }
        let token: AuthTokens =
            serde_json::from_value(value).map_err(|e| ApiError::Auth(format!("token: {e}")))?;
        Ok(PollOutcome::Token(token))
    }

    /// Run the full device flow, calling `on_code` with the code to display, then
    /// polling until the user confirms or `timeout` elapses.
    pub async fn device_auth<F>(&self, on_code: F) -> Result<AuthTokens, ApiError>
    where
        F: FnOnce(&DeviceCode),
    {
        let device_id = crate::config::device_id();
        let code = self
            .request_device_code(&device_id, "yandex-music-native")
            .await?;
        on_code(&code);

        let interval = Duration::from_secs(code.interval.max(3) as u64);
        let deadline = std::time::Instant::now() + Duration::from_secs(code.expires_in as u64);
        loop {
            tokio::time::sleep(interval).await;
            if std::time::Instant::now() >= deadline {
                return Err(ApiError::Auth("device authorization timed out".into()));
            }
            match self.poll_device_token(&code.device_code).await? {
                PollOutcome::Pending => {}
                PollOutcome::Token(token) => {
                    self.set_tokens(Some(token.clone()));
                    return Ok(token);
                }
            }
        }
    }

    /// Exchange a refresh token for a fresh access token.
    pub async fn refresh_access_token(&self, refresh_token: &str) -> Result<AuthTokens, ApiError> {
        let form = [
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
            ("client_id", crate::api::CLIENT_ID.to_string()),
            ("client_secret", crate::api::CLIENT_SECRET.to_string()),
        ];
        let response = self
            .http
            .post(format!("{OAUTH_BASE_URL}/token"))
            .form(&form)
            .send()
            .await?;
        let status = response.status();
        let value: serde_json::Value = response.json().await?;
        if !status.is_success() {
            return Err(ApiError::Auth(format!(
                "refresh failed: {}",
                value
                    .get("error_description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
            )));
        }
        let token: AuthTokens =
            serde_json::from_value(value).map_err(|e| ApiError::Auth(format!("token: {e}")))?;
        Ok(token)
    }

    /// If a saved token exists, ensure it is fresh (refresh on auth failure).
    ///
    /// Returns `true` when a usable token is installed.
    pub async fn ensure_authenticated(&self) -> Result<bool, ApiError> {
        let Some(tokens) = self.tokens() else {
            return Ok(false);
        };
        if let Err(err) = self.account_status().await {
            if err.is_auth() {
                let refresh = tokens
                    .refresh_token
                    .clone()
                    .ok_or_else(|| ApiError::Auth("no refresh token; re-login required".into()))?;
                let refreshed = self.refresh_access_token(&refresh).await?;
                self.set_tokens(Some(refreshed));
                return Ok(true);
            }
            return Err(err);
        }
        Ok(true)
    }
}

impl AuthTokens {
    /// Whether the current access token is a placeholder (never set).
    pub fn is_set(&self) -> bool {
        !self.access_token.is_empty()
    }
}

/// Convenience used by callers that want to persist a fresh token pair.
pub fn token_json(token: &AuthTokens) -> serde_json::Value {
    json!({
        "access_token": token.access_token,
        "refresh_token": token.refresh_token,
        "expires_in": token.expires_in,
    })
}
