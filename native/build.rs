//! Exposes the bundled addon's `## Version:` (from its .toc) as `WCT_ADDON_VERSION`.

use std::path::Path;

fn main() {
    let dir = Path::new("..").join("addon").join("WoWChatTranslator");
    let toc_path = dir.join("WoWChatTranslator.toc");
    println!("cargo:rerun-if-changed={}", toc_path.display());
    println!("cargo:rerun-if-changed={}", dir.join("WoWChatTranslator.lua").display());
    let toc = std::fs::read_to_string(&toc_path).expect("addon .toc must exist next to the crate");
    let version = toc
        .lines()
        .map(|l| l.trim_start_matches('\u{feff}').trim())
        .find_map(|l| {
            let rest = l.strip_prefix("##")?.trim_start();
            let (key, value) = rest.split_once(':')?;
            key.trim().eq_ignore_ascii_case("version").then(|| value.trim().to_string())
        })
        .filter(|v| !v.is_empty())
        .expect("addon .toc must have a ## Version: line");
    println!("cargo:rustc-env=WCT_ADDON_VERSION={version}");
}
