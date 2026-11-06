//! Finds the WoW installs on this PC and the game flavors inside them.
//!
//! A WoW *root* (e.g. `C:\Program Files (x86)\World of Warcraft`) holds one folder per
//! installed flavor (`_retail_`, `_classic_`, `_classic_era_`, `_forever_`, ...), each
//! with its own game executable (`Wow.exe`, `WowClassic.exe`, `WowClassicT.exe`, ...)
//! and its own `Interface\AddOns`. Roots come from the Blizzard registry key, the
//! uninstall entries, the folder of a running game and folders the user picked.
//! Registry and process access go through [`WowSystem`] so the logic is testable;
//! folders are read from the real filesystem.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{toc_version, ADDON_DIR};

/// One `...\CurrentVersion\Uninstall\<key>` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UninstallEntry {
    pub key: String,
    pub display_name: Option<String>,
    pub install_location: Option<String>,
}

/// Where detection reads the registry and the running processes.
pub trait WowSystem {
    /// `HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft\InstallPath`.
    fn install_path(&self) -> Option<String>;
    /// The uninstall entries (any program; filtering happens here).
    fn uninstall_entries(&self) -> Vec<UninstallEntry>;
    /// Full paths of running game clients (`Wow*.exe`).
    fn running_wow_exes(&self) -> Vec<PathBuf>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootSource {
    Registry,
    Uninstall,
    Process,
    User,
}

/// The addon folder in one flavor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddonState {
    Missing,
    Installed { version: Option<String> },
    /// A junction or symlink (a development copy); never written through.
    Linked { target: Option<PathBuf>, version: Option<String> },
}

impl AddonState {
    pub fn version(&self) -> Option<&str> {
        match self {
            AddonState::Missing => None,
            AddonState::Installed { version } | AddonState::Linked { version, .. } => version.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flavor {
    /// Folder name, e.g. `_classic_era_`.
    pub folder: String,
    /// Name shown to the user, e.g. "Classic Era".
    pub name: String,
    pub path: PathBuf,
    pub addon: AddonState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WowRoot {
    pub path: PathBuf,
    pub sources: Vec<RootSource>,
    pub flavors: Vec<Flavor>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detection {
    pub roots: Vec<WowRoot>,
    /// Flavor folder the launcher last started (registry), else a running one.
    pub last_played: Option<PathBuf>,
}

impl Detection {
    pub fn flavors(&self) -> impl Iterator<Item = &Flavor> {
        self.roots.iter().flat_map(|r| r.flavors.iter())
    }

    pub fn is_last_played(&self, flavor: &Flavor) -> bool {
        self.last_played.as_ref().is_some_and(|p| same_path(p, &flavor.path))
    }

    pub fn last_played_flavor(&self) -> Option<&Flavor> {
        self.flavors().find(|f| self.is_last_played(f))
    }
}

/// Case-insensitive key of a path: `/` as `\`, no trailing separator.
pub fn normalize_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn same_path(a: &Path, b: &Path) -> bool {
    normalize_key(a) == normalize_key(b)
}

/// A game client executable, matched by an explicit allow-list shape
/// `wow(classic)?[a-z]?.exe` (case-insensitive): `Wow.exe`, `WowT.exe`, `WowB.exe`,
/// `WowClassic.exe`, `WowClassicT.exe`, `WowClassicB.exe`... A prefix match is not
/// enough: tools like `WowUp.exe` (addon manager), `WowVoiceProxy.exe` or this app
/// (`wow-chat-translator.exe`) must never be taken for the game.
pub fn is_wow_client_exe(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".exe").and_then(|s| s.strip_prefix("wow")) else {
        return false;
    };
    let rest = stem.strip_prefix("classic").unwrap_or(stem);
    rest.len() <= 1 && rest.chars().all(|c| c.is_ascii_lowercase())
}

/// `_xxx_` folder names (`_retail_`, `_classic_era_`...).
pub fn is_flavor_folder_name(name: &str) -> bool {
    name.len() > 2 && name.starts_with('_') && name.ends_with('_')
}

/// Name shown for a flavor folder when `.build.info` says nothing about it.
pub fn friendly_name(folder: &str) -> String {
    match folder {
        "_forever_" => "WoW Forever",
        "_retail_" => "Retail",
        "_classic_" => "Classic",
        "_classic_era_" => "Classic Era",
        "_anniversary_" => "Anniversary",
        "_classic_ptr_" => "Classic PTR",
        "_classic_beta_" => "Classic Beta",
        "_ptr_" => "PTR",
        "_beta_" => "Beta",
        other => other,
    }
    .to_string()
}

/// Flavor folder of a Blizzard product code (`wow` → `_retail_`, `wow_classic_era` →
/// `_classic_era_`, `wowt` → `_ptr_`, `wowf` → `_forever_`).
pub fn folder_for_product(product: &str) -> String {
    match product {
        "wow" => "_retail_".into(),
        "wowt" => "_ptr_".into(),
        "wowf" => "_forever_".into(),
        "wowxptr" => "_xptr_".into(),
        "wowz" => "_submission_".into(),
        _ => format!("_{}_", product.strip_prefix("wow_").unwrap_or(product)),
    }
}

/// Name of a Blizzard product at a game version: `wow_classic` 5.x is "Mists of
/// Pandaria Classic", `wow_classic_ptr` adds " PTR", and so on. `None` when unknown.
pub fn product_name(product: &str, version: &str) -> Option<String> {
    let (base, suffix) = match product {
        "wowt" => ("wow", " PTR"),
        "wowxptr" => ("wow", " XPTR"),
        p => match (p.strip_suffix("_ptr"), p.strip_suffix("_beta")) {
            (Some(b), _) => (b, " PTR"),
            (_, Some(b)) => (b, " Beta"),
            _ => (p, ""),
        },
    };
    let major: u32 = version.split('.').next()?.trim().parse().ok()?;
    let name = match base {
        "wow" => "Retail",
        "wowf" => "WoW Forever",
        "wow_classic_era" => "Classic Era",
        "wow_anniversary" => "Anniversary",
        "wow_classic" => match major {
            2 => "Burning Crusade Classic",
            3 => "Wrath Classic",
            4 => "Cataclysm Classic",
            5 => "Mists of Pandaria Classic",
            _ => "Classic",
        },
        _ => return None,
    };
    Some(format!("{name}{suffix}"))
}

/// (product, version) rows of a `.build.info` (pipe-separated, header `Name!TYPE:n`).
pub fn parse_build_info(text: &str) -> Vec<(String, String)> {
    let mut lines = text.lines().map(|l| l.trim_start_matches('\u{feff}').trim_end());
    let Some(header) = lines.next() else { return Vec::new() };
    let columns: Vec<&str> = header.split('|').map(|c| c.split('!').next().unwrap_or("").trim()).collect();
    let find = |name: &str| columns.iter().position(|c| c.eq_ignore_ascii_case(name));
    let (Some(vi), Some(pi)) = (find("Version"), find("Product")) else { return Vec::new() };
    lines
        .filter_map(|l| {
            let cells: Vec<&str> = l.split('|').collect();
            let product = cells.get(pi)?.trim();
            let version = cells.get(vi)?.trim();
            (!product.is_empty() && !version.is_empty()).then(|| (product.to_string(), version.to_string()))
        })
        .collect()
}

/// Flavor folder → name, from `<root>\.build.info` when it can be read.
fn build_info_names(root: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(root.join(".build.info")) else { return HashMap::new() };
    parse_build_info(&text)
        .into_iter()
        .filter_map(|(product, version)| Some((folder_for_product(&product), product_name(&product, &version)?)))
        .collect()
}

fn has_client_exe(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| e.file_type().is_ok_and(|t| t.is_file()) && is_wow_client_exe(&e.file_name().to_string_lossy()))
    })
}

fn is_flavor_dir(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| is_flavor_folder_name(&n.to_string_lossy())) && dir.is_dir() && has_client_exe(dir)
}

/// `<flavor>\Interface\AddOns\WoWChatTranslator`.
pub fn addon_dir(flavor: &Path) -> PathBuf {
    flavor.join("Interface").join("AddOns").join(ADDON_DIR)
}

fn read_toc_version(dir: &Path) -> Option<String> {
    let raw = fs::read(dir.join(format!("{ADDON_DIR}.toc"))).ok()?;
    toc_version(&String::from_utf8_lossy(&raw))
}

/// Drops the `\\?\` / `\??\` prefix Windows puts on link targets.
fn clean_link_target(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    for prefix in [r"\\?\UNC\", r"\\?\", r"\??\"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return PathBuf::from(if prefix.ends_with(r"UNC\") { format!(r"\\{rest}") } else { rest.to_string() });
        }
    }
    p
}

/// What is at `<flavor>\Interface\AddOns\WoWChatTranslator` (links are not followed
/// to decide, only to read the version).
pub fn addon_state(flavor: &Path) -> AddonState {
    let dir = addon_dir(flavor);
    match fs::symlink_metadata(&dir) {
        Err(_) => AddonState::Missing,
        Ok(meta) if meta.file_type().is_symlink() => AddonState::Linked {
            target: fs::read_link(&dir).ok().map(clean_link_target),
            version: read_toc_version(&dir),
        },
        Ok(meta) if meta.is_dir() => AddonState::Installed { version: read_toc_version(&dir) },
        Ok(_) => AddonState::Missing,
    }
}

const FLAVOR_ORDER: [&str; 9] = [
    "_retail_",
    "_classic_",
    "_classic_era_",
    "_anniversary_",
    "_forever_",
    "_ptr_",
    "_classic_ptr_",
    "_beta_",
    "_classic_beta_",
];

/// The flavors of a root: `_xxx_` folders holding a game executable (an empty
/// `_retail_` left behind by the launcher is not installed and is skipped).
pub fn list_flavors(root: &Path) -> Vec<Flavor> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let names = build_info_names(root);
    let mut flavors: Vec<Flavor> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_flavor_dir(p))
        .map(|path| {
            let folder = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let name = names.get(&folder.to_lowercase()).cloned().unwrap_or_else(|| friendly_name(&folder.to_lowercase()));
            let addon = addon_state(&path);
            Flavor { folder, name, path, addon }
        })
        .collect();
    let rank = |f: &Flavor| FLAVOR_ORDER.iter().position(|o| o.eq_ignore_ascii_case(&f.folder)).unwrap_or(FLAVOR_ORDER.len());
    flavors.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.folder.to_lowercase().cmp(&b.folder.to_lowercase())));
    flavors
}

fn contains_flavors(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| is_flavor_dir(&e.path())))
}

/// A registry path that may point at a flavor folder: (root, flavor if any).
fn split_flavor(path: &Path) -> (PathBuf, Option<PathBuf>) {
    let is_flavor = path.file_name().is_some_and(|n| is_flavor_folder_name(&n.to_string_lossy()));
    match (is_flavor, path.parent()) {
        (true, Some(parent)) => (parent.to_path_buf(), Some(path.to_path_buf())),
        _ => (path.to_path_buf(), None),
    }
}

/// The WoW root for a folder the user picked: the root itself, one of its flavors or
/// a folder inside a flavor (e.g. `Interface\AddOns`). `None` when no WoW is there.
pub fn resolve_user_folder(folder: &Path) -> Option<PathBuf> {
    for dir in folder.ancestors().take(5) {
        if is_flavor_dir(dir) {
            return dir.parent().map(Path::to_path_buf);
        }
        if contains_flavors(dir) {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// Uninstall entries of WoW: "World of Warcraft ..." or a Classic game's own name.
pub fn is_wow_program(entry: &UninstallEntry) -> bool {
    const CLASSIC_NAMES: [&str; 5] = [
        "burning crusade classic",
        "wrath of the lich king classic",
        "cataclysm classic",
        "mists of pandaria classic",
        "wow classic",
    ];
    [entry.display_name.as_deref(), Some(entry.key.as_str())].into_iter().flatten().any(|name| {
        let n = name.to_lowercase();
        n.contains("world of warcraft") || CLASSIC_NAMES.iter().any(|c| n.contains(c))
    })
}

/// Finds every WoW root (deduplicated, existing folders only) and its flavors.
pub fn detect(sys: &dyn WowSystem, user_folders: &[PathBuf]) -> Detection {
    let mut candidates: Vec<(PathBuf, RootSource)> = Vec::new();
    let mut last_played = None;

    if let Some(install) = sys.install_path().filter(|s| !s.trim().is_empty()) {
        let (root, flavor) = split_flavor(Path::new(install.trim()));
        last_played = flavor;
        candidates.push((root, RootSource::Registry));
    }
    for entry in sys.uninstall_entries().iter().filter(|e| is_wow_program(e)) {
        if let Some(location) = entry.install_location.as_deref().filter(|s| !s.trim().is_empty()) {
            candidates.push((split_flavor(Path::new(location.trim())).0, RootSource::Uninstall));
        }
    }
    let mut running_flavor = None;
    for exe in sys.running_wow_exes() {
        let Some(dir) = exe.parent() else { continue };
        let (root, flavor) = split_flavor(dir);
        running_flavor = running_flavor.or(flavor);
        candidates.push((root, RootSource::Process));
    }
    for folder in user_folders {
        if let Some(root) = resolve_user_folder(folder) {
            candidates.push((root, RootSource::User));
        }
    }

    let mut roots: Vec<WowRoot> = Vec::new();
    for (path, source) in candidates {
        if !path.is_dir() {
            continue;
        }
        let key = normalize_key(&path);
        match roots.iter_mut().find(|r| normalize_key(&r.path) == key) {
            Some(root) if !root.sources.contains(&source) => root.sources.push(source),
            Some(_) => {}
            None => roots.push(WowRoot { flavors: list_flavors(&path), path, sources: vec![source] }),
        }
    }

    let mut detection = Detection { roots, last_played: None };
    for candidate in [last_played, running_flavor].into_iter().flatten() {
        detection.last_played = Some(candidate);
        if detection.last_played_flavor().is_some() {
            return detection;
        }
    }
    detection.last_played = None;
    detection
}
