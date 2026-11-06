//! Addon-related choices, kept in their own `addon.json` next to `config.json`:
//! whether the first-run dialog was answered and the WoW folders the user picked.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddonPrefs {
    /// The first-run dialog was answered ("Instalar addon" or "Ahora no").
    pub addon_prompt_done: bool,
    /// Folders chosen with "Elegir otra carpeta…".
    pub extra_folders: Vec<PathBuf>,
}

/// Missing or unreadable file: defaults.
pub fn load(path: &Path) -> AddonPrefs {
    std::fs::read_to_string(path).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

pub fn save(path: &Path, prefs: &AddonPrefs) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(prefs).map_err(io::Error::other)?;
    std::fs::write(path, json)
}
