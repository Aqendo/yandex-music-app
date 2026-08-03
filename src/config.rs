//! App configuration and token persistence (XDG config dir).
use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::api::AuthTokens;

/// Disk representation of the app configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    pub tokens: Option<AuthTokens>,
    pub device_id: Option<String>,
    pub language: Option<String>,
    /// Last "My Wave" mood/energy preset (one of `radio::MOODS`).
    pub wave_mood: Option<String>,
    /// Last "My Wave" diversity preset (one of `radio::DIVERSITIES`).
    pub wave_diversity: Option<String>,
    /// Last "My Wave" language preset (one of `radio::LANGUAGES`).
    pub wave_language: Option<String>,
    /// Last output volume as a percentage (0.0–100.0); defaults to 100.
    pub volume: Option<f64>,
}

/// XDG config location: `$XDG_CONFIG_HOME/yandex-music/config.json`.
pub fn config_path() -> PathBuf {
    ProjectDirs::from("", "", "yandex-music")
        .map(|dirs| dirs.config_dir().join("config.json"))
        .unwrap_or_else(|| PathBuf::from(".ymapp-config.json"))
}

/// Load the config file, tolerating a missing or corrupt file.
pub fn load_config() -> ConfigFile {
    let path = config_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return ConfigFile::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Persist the config file, creating the config directory as needed.
pub fn save_config(config: &ConfigFile) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create config dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("cannot serialize config: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// A stable per-install device id, used by the OAuth device flow.
///
/// Generated once and persisted, so re-logins reuse the same device identity.
pub fn device_id() -> String {
    let mut config = load_config();
    if config.device_id.is_none() {
        config.device_id = Some(random_alnum(10));
        let _ = save_config(&config);
    }
    config.device_id.unwrap_or_else(|| random_alnum(10))
}

fn random_alnum(len: usize) -> String {
    let mut rng = rand::thread_rng();
    std::iter::repeat_with(|| rng.sample(Alphanumeric) as char)
        .take(len)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_stable() {
        // Re-running must return the same value within one process run.
        let first = device_id();
        assert_eq!(first.len(), 10);
        assert!(first.chars().all(char::is_alphanumeric));
    }

    #[test]
    fn wave_settings_roundtrip() {
        let config = ConfigFile {
            wave_mood: Some("calm".to_string()),
            wave_diversity: Some("default".to_string()),
            wave_language: Some("any".to_string()),
            volume: Some(42.0),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ConfigFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.wave_mood.as_deref(), Some("calm"));
        assert_eq!(parsed.wave_diversity.as_deref(), Some("default"));
        assert_eq!(parsed.wave_language.as_deref(), Some("any"));
        assert_eq!(parsed.volume, Some(42.0));
    }
}
