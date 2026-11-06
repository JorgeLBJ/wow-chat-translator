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

    #[cfg(windows)]
    embed_windows_resources();
}

/// Embeds the app icon and version info into the Windows executable.
#[cfg(windows)]
fn embed_windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = Path::new("assets").join("icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let mut res = winresource::WindowsResource::new();
    res.set_icon(&icon.to_string_lossy())
        .set("ProductName", "WoW Chat Translator")
        .set("FileDescription", "WoW Chat Translator")
        .set("CompanyName", "Jorge Luis Bustamante Jara")
        .set("LegalCopyright", "Copyright (c) 2026 Jorge Luis Bustamante Jara")
        .set("OriginalFilename", "wow-chat-translator.exe")
        .set("InternalName", "wow-chat-translator");
    res.compile().expect("failed to embed Windows resources (icon, version info)");
}
