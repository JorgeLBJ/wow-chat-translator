//! The WoW addon bundled inside the app: its files, its version, finding the WoW
//! installs (`detect`) and installing or updating the addon into them (`install`).

pub mod detect;
pub mod hint;
pub mod install;
pub mod prefs;
#[cfg(windows)]
pub mod system;

use std::cmp::Ordering;

use detect::AddonState;

/// Folder name under `Interface\AddOns`.
pub const ADDON_DIR: &str = "WoWChatTranslator";
/// `## Version:` of the bundled .toc (read by build.rs).
pub const VERSION: &str = env!("WCT_ADDON_VERSION");

const TOC: &[u8] = include_bytes!("../../../addon/WoWChatTranslator/WoWChatTranslator.toc");
const LUA: &[u8] = include_bytes!("../../../addon/WoWChatTranslator/WoWChatTranslator.lua");

/// The addon files, as (file name, bytes), in install order. The .toc carries the
/// version, so it is written LAST: if writing the .lua fails, the folder never claims
/// the new version with the old code (and the update is offered again).
pub fn files() -> [(&'static str, &'static [u8]); 2] {
    [("WoWChatTranslator.lua", LUA), ("WoWChatTranslator.toc", TOC)]
}

/// The `## Version:` value of a .toc text (key case-insensitive), if any.
pub fn toc_version(toc: &str) -> Option<String> {
    toc.lines()
        .map(|l| l.trim_start_matches('\u{feff}').trim())
        .find_map(|l| {
            let rest = l.strip_prefix("##")?.trim_start();
            let (key, value) = rest.split_once(':')?;
            key.trim().eq_ignore_ascii_case("version").then(|| value.trim().to_string())
        })
        .filter(|v| !v.is_empty())
}

fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.trim().trim_start_matches(['v', 'V']).split('.').map(|p| p.trim().parse().ok()).collect()
}

/// True when `installed` is older than `bundled`. A missing or unreadable installed
/// version counts as older; missing trailing parts count as 0 ("0.2" == "0.2.0").
pub fn is_older(installed: Option<&str>, bundled: &str) -> bool {
    let Some(b) = parse_version(bundled) else { return false };
    let Some(i) = installed.and_then(parse_version) else { return true };
    let len = i.len().max(b.len());
    let part = |v: &Vec<u64>, k: usize| v.get(k).copied().unwrap_or(0);
    for k in 0..len {
        match part(&i, k).cmp(&part(&b, k)) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
    }
    false
}

/// The addon in a flavor should be (re)installed: missing or older than `bundled`.
/// A link to a development copy is left to the developer.
pub fn needs_update(state: &AddonState, bundled: &str) -> bool {
    match state {
        AddonState::Missing => true,
        AddonState::Installed { version } => is_older(version.as_deref(), bundled),
        AddonState::Linked { .. } => false,
    }
}
