//! Persisted settings, compatible with the Electron app's `config.json`
//! (`prefs`, `translator`, `overlayBounds`). Missing keys take their defaults, unknown
//! keys are ignored, and a missing or unreadable file means all defaults.
//! The API key stays as stored (`apiKeyEnc`); encryption lives elsewhere.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub font_size: f32,
    pub opacity: f32,
    pub always_on_top: bool,
    pub auto_start: bool,
    pub translation_paused: bool,
    /// Keep the windows out of screenshots and recordings (streams). Off by default.
    pub hide_from_capture: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { font_size: 13.0, opacity: 1.0, always_on_top: true, auto_start: true, translation_paused: false, hide_from_capture: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TranslatorSettings {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    /// Encrypted API key (base64), only valid for `key_binding`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_enc: Option<String>,
    /// Provider + server the saved key belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_binding: Option<String>,
}

impl Default for TranslatorSettings {
    fn default() -> Self {
        Self { provider: "google".into(), base_url: String::new(), model: String::new(), api_key_enc: None, key_binding: None }
    }
}

/// Last overlay position and size; any part may be missing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bounds {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub prefs: Prefs,
    pub translator: TranslatorSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_bounds: Option<Bounds>,
}

/// Reads the config; a missing or unreadable file gives the defaults.
pub fn load(path: &Path) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Writes the config as pretty JSON, creating the folder if needed.
pub fn save(path: &Path, config: &Config) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    std::fs::write(path, json)
}
