//! Addon distribution: embedded files, WoW detection with fake providers, flavor
//! listing and names, .toc versions, install into temp folders, junction safety.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use wow_chat_translator::addon::detect::{
    detect, folder_for_product, friendly_name, is_wow_client_exe, list_flavors, parse_build_info, product_name,
    AddonState, RootSource, UninstallEntry, WowSystem,
};
use wow_chat_translator::addon::hint::UpdateHint;
use wow_chat_translator::addon::install::{install, InstallResult};
use wow_chat_translator::addon::prefs::{self as addon_prefs, AddonPrefs};
use wow_chat_translator::addon::{self, is_older, needs_update, toc_version, ADDON_DIR};

// Helpers -----------------------------------------------------------------------------------

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh folder under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("wct-addon-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `<root>\<folder>\<exe>` (an empty file standing for the game).
fn make_flavor(root: &Path, folder: &str, exe: &str) -> PathBuf {
    let dir = root.join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(exe), b"").unwrap();
    dir
}

fn write_installed(flavor: &Path, version: &str) -> PathBuf {
    let dir = flavor.join("Interface").join("AddOns").join(ADDON_DIR);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("WoWChatTranslator.toc"), format!("## Interface: 16001\r\n## Version: {version}\r\n")).unwrap();
    dir
}

#[derive(Default)]
struct FakeSystem {
    install_path: Option<String>,
    uninstall: Vec<UninstallEntry>,
    exes: Vec<PathBuf>,
}

impl WowSystem for FakeSystem {
    fn install_path(&self) -> Option<String> {
        self.install_path.clone()
    }
    fn uninstall_entries(&self) -> Vec<UninstallEntry> {
        self.uninstall.clone()
    }
    fn running_wow_exes(&self) -> Vec<PathBuf> {
        self.exes.clone()
    }
}

fn entry(key: &str, name: Option<&str>, location: &Path) -> UninstallEntry {
    UninstallEntry {
        key: key.into(),
        display_name: name.map(str::to_string),
        install_location: Some(location.display().to_string()),
    }
}

fn folders(d: &wow_chat_translator::addon::detect::Detection) -> Vec<String> {
    d.flavors().map(|f| f.folder.clone()).collect()
}

const BUILD_INFO: &str = "Branch!STRING:0|Active!DEC:1|Build Key!HEX:16|Version!STRING:0|Product!STRING:0\r\n\
us|1|aa|5.5.4.70631|wow_classic\r\n\
us|1|bb|1.15.9.70576|wow_classic_era\r\n\
us|1|cc|5.5.4.67849|wow_classic_ptr\r\n\
us|1|dd|1.60.1.70790|wowf\r\n";

// Embedded addon -------------------------------------------------------------------------------

#[test]
fn embedded_files_equal_the_repo_files_byte_for_byte() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("addon").join(ADDON_DIR);
    let files = addon::files();
    assert_eq!(files.len(), 2);
    for (name, bytes) in files {
        let on_disk = fs::read(dir.join(name)).unwrap();
        assert_eq!(bytes, on_disk.as_slice(), "{name} differs from the repo copy");
    }
}

#[test]
fn bundled_version_is_the_toc_version() {
    let toc = std::str::from_utf8(embedded("WoWChatTranslator.toc")).unwrap();
    assert_eq!(toc_version(toc).as_deref(), Some(addon::VERSION));
    assert_eq!(addon::VERSION, "0.3.3");
}

#[test]
fn app_addon_and_installer_share_one_version_number() {
    // One number for everything: the app (Cargo.toml), the bundled addon (.toc) and the
    // installer (.iss AppVersion). Bump all three together.
    let app = env!("CARGO_PKG_VERSION");
    assert_eq!(addon::VERSION, app, "addon .toc ## Version must equal Cargo.toml version");
    let iss = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../installer/wow-chat-translator.iss")).unwrap();
    let installer = iss
        .lines()
        .find_map(|l| l.trim().strip_prefix("#define AppVersion "))
        .map(|v| v.trim().trim_matches('"').to_string());
    assert_eq!(installer.as_deref(), Some(app), "installer AppVersion must equal Cargo.toml version");
}

#[test]
fn toc_lists_wow_forever_classic_era_and_mists_classic() {
    let toc = std::str::from_utf8(embedded("WoWChatTranslator.toc")).unwrap();
    let line = toc.lines().find(|l| l.starts_with("## Interface:")).unwrap();
    let numbers: Vec<&str> = line["## Interface:".len()..].split(',').map(str::trim).collect();
    assert_eq!(numbers[0], "16001");
    assert!(numbers.contains(&"11509"));
    assert!(numbers.contains(&"50504"));
}

// Versions ---------------------------------------------------------------------------------------

#[test]
fn toc_version_parsing() {
    assert_eq!(toc_version("## Interface: 16001\r\n## Version: 0.1.0\r\n").as_deref(), Some("0.1.0"));
    assert_eq!(toc_version("\u{feff}## Version:   1.2.3  \n").as_deref(), Some("1.2.3"));
    assert_eq!(toc_version("## version: 2.0\n").as_deref(), Some("2.0"));
    assert_eq!(toc_version("## Title: X\n"), None);
    assert_eq!(toc_version("## Version:\n"), None);
}

#[test]
fn version_comparison() {
    assert!(is_older(Some("0.1.0"), "0.2.0"));
    assert!(is_older(Some("0.2"), "0.2.1"));
    assert!(!is_older(Some("0.2.0"), "0.2.0"));
    assert!(!is_older(Some("0.2"), "0.2.0"));
    assert!(!is_older(Some("0.10.0"), "0.2.0"));
    assert!(is_older(None, "0.2.0"));
    assert!(is_older(Some("dev"), "0.2.0"));
}

#[test]
fn update_needed_only_for_missing_or_older_copies() {
    assert!(needs_update(&AddonState::Missing, "0.2.0"));
    assert!(needs_update(&AddonState::Installed { version: Some("0.1.0".into()) }, "0.2.0"));
    assert!(!needs_update(&AddonState::Installed { version: Some("0.2.0".into()) }, "0.2.0"));
    // A development link is the developer's business.
    assert!(!needs_update(&AddonState::Linked { target: None, version: Some("0.1.0".into()) }, "0.2.0"));
}

#[test]
fn update_hint_fires_once_after_a_while_without_strip() {
    let mut h = UpdateHint::new(30_000);
    assert!(!h.due(100_000), "no game, no hint");
    h.game_started(1_000);
    assert!(!h.due(20_000));
    assert!(h.due(31_000));
    assert!(!h.due(40_000), "checked once per session");
    h.game_exited();
    h.game_started(50_000);
    h.strip_seen();
    assert!(!h.due(90_000), "the strip was seen: the addon works");
}

// Names ------------------------------------------------------------------------------------------

#[test]
fn friendly_names_from_folders() {
    for (folder, name) in [
        ("_forever_", "WoW Forever"),
        ("_retail_", "Retail"),
        ("_classic_", "Classic"),
        ("_classic_era_", "Classic Era"),
        ("_anniversary_", "Anniversary"),
        ("_classic_ptr_", "Classic PTR"),
        ("_classic_beta_", "Classic Beta"),
        ("_ptr_", "PTR"),
        ("_beta_", "Beta"),
        ("_xptr_", "_xptr_"),
    ] {
        assert_eq!(friendly_name(folder), name);
    }
}

#[test]
fn product_names_from_build_info() {
    assert_eq!(product_name("wow_classic", "5.5.4.70631").as_deref(), Some("Mists of Pandaria Classic"));
    assert_eq!(product_name("wow_classic", "4.4.2.1").as_deref(), Some("Cataclysm Classic"));
    assert_eq!(product_name("wow_classic", "3.4.3.1").as_deref(), Some("Wrath Classic"));
    assert_eq!(product_name("wow_classic", "2.5.4.1").as_deref(), Some("Burning Crusade Classic"));
    assert_eq!(product_name("wow_classic_era", "1.15.9.70576").as_deref(), Some("Classic Era"));
    assert_eq!(product_name("wowf", "1.60.1.70790").as_deref(), Some("WoW Forever"));
    assert_eq!(product_name("wow", "12.0.5.1").as_deref(), Some("Retail"));
    assert_eq!(product_name("wow_classic_ptr", "5.5.4.67849").as_deref(), Some("Mists of Pandaria Classic PTR"));
    assert_eq!(product_name("wow_classic_beta", "5.5.0.1").as_deref(), Some("Mists of Pandaria Classic Beta"));
    assert_eq!(product_name("wow_classic_era_ptr", "1.15.9.1").as_deref(), Some("Classic Era PTR"));
    assert_eq!(product_name("wowt", "12.0.5.1").as_deref(), Some("Retail PTR"));
    assert_eq!(product_name("agent", "1.0"), None);
}

#[test]
fn product_folders() {
    assert_eq!(folder_for_product("wow"), "_retail_");
    assert_eq!(folder_for_product("wowt"), "_ptr_");
    assert_eq!(folder_for_product("wowf"), "_forever_");
    assert_eq!(folder_for_product("wow_beta"), "_beta_");
    assert_eq!(folder_for_product("wow_classic"), "_classic_");
    assert_eq!(folder_for_product("wow_classic_era"), "_classic_era_");
    assert_eq!(folder_for_product("wow_classic_ptr"), "_classic_ptr_");
}

#[test]
fn build_info_parsing() {
    let rows = parse_build_info(BUILD_INFO);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0], ("wow_classic".to_string(), "5.5.4.70631".to_string()));
    assert_eq!(rows[3], ("wowf".to_string(), "1.60.1.70790".to_string()));
    assert!(parse_build_info("garbage").is_empty());
}

#[test]
fn game_client_executables() {
    for exe in ["Wow.exe", "wow.exe", "WowClassic.exe", "WowClassicT.exe", "WowClassicB.exe", "WowT.exe", "WowB.exe"] {
        assert!(is_wow_client_exe(exe), "{exe}");
    }
    for exe in ["WowVoiceProxy.exe", "wow-chat-translator.exe", "WowError.exe", "World of Warcraft Launcher.exe", "Wow.dll", "Wow", "WowUp.exe", "WowUpCf.exe", "WowheadClient.exe", "WowClassicHelper.exe", "Wowlauncher.exe"] {
        assert!(!is_wow_client_exe(exe), "{exe}");
    }
}

// Flavors -----------------------------------------------------------------------------------------

#[test]
fn flavors_are_flavor_folders_with_a_game_executable() {
    let t = TempDir::new();
    make_flavor(t.path(), "_forever_", "Wow.exe");
    make_flavor(t.path(), "_classic_", "WowClassic.exe");
    make_flavor(t.path(), "_classic_ptr_", "WowClassicT.exe");
    fs::create_dir_all(t.path().join("_retail_").join("Interface")).unwrap(); // empty: not installed
    make_flavor(t.path(), "Data", "Wow.exe"); // not a _flavor_ folder
    make_flavor(t.path(), "_classic_era_", "WowVoiceProxy.exe"); // only a helper
    let flavors = list_flavors(t.path());
    let mut names: Vec<&str> = flavors.iter().map(|f| f.folder.as_str()).collect();
    names.sort();
    assert_eq!(names, ["_classic_", "_classic_ptr_", "_forever_"]);
    assert!(flavors.iter().all(|f| f.addon == AddonState::Missing));
}

#[test]
fn flavor_names_prefer_build_info() {
    let t = TempDir::new();
    make_flavor(t.path(), "_classic_", "WowClassic.exe");
    make_flavor(t.path(), "_classic_beta_", "WowClassicB.exe");
    fs::write(t.path().join(".build.info"), BUILD_INFO).unwrap();
    let flavors = list_flavors(t.path());
    let name = |folder: &str| flavors.iter().find(|f| f.folder == folder).unwrap().name.clone();
    assert_eq!(name("_classic_"), "Mists of Pandaria Classic");
    assert_eq!(name("_classic_beta_"), "Classic Beta", "no build.info row: folder name");
}

#[test]
fn installed_version_is_reported_per_flavor() {
    let t = TempDir::new();
    let f = make_flavor(t.path(), "_forever_", "Wow.exe");
    write_installed(&f, "0.1.0");
    let flavors = list_flavors(t.path());
    assert_eq!(flavors[0].addon, AddonState::Installed { version: Some("0.1.0".into()) });
}

// Detection -----------------------------------------------------------------------------------------

#[test]
fn registry_install_path_with_flavor_gives_root_and_last_played() {
    let t = TempDir::new();
    let forever = make_flavor(t.path(), "_forever_", "Wow.exe");
    make_flavor(t.path(), "_classic_", "WowClassic.exe");
    let sys = FakeSystem { install_path: Some(format!("{}\\", forever.display())), ..Default::default() };
    let d = detect(&sys, &[]);
    assert_eq!(d.roots.len(), 1);
    assert_eq!(d.roots[0].sources, [RootSource::Registry]);
    assert_eq!(folders(&d).len(), 2);
    assert_eq!(d.last_played_flavor().map(|f| f.folder.as_str()), Some("_forever_"));
}

#[test]
fn uninstall_entries_alone_find_the_root() {
    let t = TempDir::new();
    make_flavor(t.path(), "_classic_era_", "WowClassic.exe");
    let other = TempDir::new();
    make_flavor(other.path(), "_retail_", "Wow.exe");
    let sys = FakeSystem {
        uninstall: vec![
            entry("World of Warcraft Classic Era", Some("World of Warcraft Classic Era"), t.path()),
            entry("Warcraft III", Some("Warcraft III"), other.path()),
        ],
        ..Default::default()
    };
    let d = detect(&sys, &[]);
    assert_eq!(d.roots.len(), 1, "Warcraft III is not WoW");
    assert_eq!(d.roots[0].sources, [RootSource::Uninstall]);
    assert_eq!(folders(&d), ["_classic_era_"]);
    assert!(d.last_played.is_none());
}

#[test]
fn uninstall_entry_named_like_a_classic_game_counts() {
    let t = TempDir::new();
    make_flavor(t.path(), "_classic_", "WowClassic.exe");
    let sys = FakeSystem { uninstall: vec![entry("Cataclysm Classic", None, t.path())], ..Default::default() };
    assert_eq!(folders(&detect(&sys, &[])), ["_classic_"]);
}

#[test]
fn running_process_alone_finds_the_root() {
    let t = TempDir::new();
    let classic = make_flavor(t.path(), "_classic_", "WowClassic.exe");
    let sys = FakeSystem { exes: vec![classic.join("WowClassic.exe")], ..Default::default() };
    let d = detect(&sys, &[]);
    assert_eq!(d.roots.len(), 1);
    assert_eq!(d.roots[0].sources, [RootSource::Process]);
    assert_eq!(d.last_played_flavor().map(|f| f.folder.as_str()), Some("_classic_"), "running flavor when the registry has none");
}

#[test]
fn duplicate_roots_in_different_case_are_merged() {
    let t = TempDir::new();
    let forever = make_flavor(t.path(), "_forever_", "Wow.exe");
    let root = t.path().display().to_string();
    let sys = FakeSystem {
        install_path: Some(forever.display().to_string()),
        uninstall: vec![
            entry("Cataclysm Classic", Some("Cataclysm Classic"), Path::new(&root.to_uppercase())),
            entry("World of Warcraft Classic PTR", Some("World of Warcraft Classic PTR"), Path::new(&format!("{}\\", root.replace('\\', "/")))),
        ],
        exes: vec![forever.join("Wow.exe")],
    };
    let d = detect(&sys, &[]);
    assert_eq!(d.roots.len(), 1);
    assert_eq!(d.roots[0].sources, [RootSource::Registry, RootSource::Uninstall, RootSource::Process]);
    assert_eq!(folders(&d), ["_forever_"]);
}

#[test]
fn user_folder_may_be_the_root_or_a_flavor() {
    let t = TempDir::new();
    let era = make_flavor(t.path(), "_classic_era_", "WowClassic.exe");
    let d = detect(&FakeSystem::default(), &[era.clone()]);
    assert_eq!(d.roots.len(), 1);
    assert_eq!(d.roots[0].sources, [RootSource::User]);
    let d = detect(&FakeSystem::default(), &[t.path().to_path_buf()]);
    assert_eq!(folders(&d), ["_classic_era_"]);
}

#[test]
fn no_wow_installed() {
    let t = TempDir::new();
    let sys = FakeSystem {
        install_path: Some(t.path().join("missing").join("_retail_").display().to_string()),
        uninstall: vec![entry("World of Warcraft", Some("World of Warcraft"), &t.path().join("gone"))],
        ..Default::default()
    };
    let d = detect(&sys, &[]);
    assert!(d.roots.is_empty());
    assert_eq!(d.flavors().count(), 0);
    assert!(d.last_played_flavor().is_none());
}

// Install -------------------------------------------------------------------------------------------

fn addon_dir(flavor: &Path) -> PathBuf {
    flavor.join("Interface").join("AddOns").join(ADDON_DIR)
}

fn no_temp_files(dir: &Path) {
    for e in fs::read_dir(dir).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        assert!(!name.contains("wct-tmp"), "left a temp file: {name}");
    }
}

#[test]
fn install_into_a_fresh_flavor() {
    let t = TempDir::new();
    let f = make_flavor(t.path(), "_forever_", "Wow.exe");
    let r = install(&f, &addon::files(), false);
    assert_eq!(r, InstallResult::Installed);
    for (name, bytes) in addon::files() {
        assert_eq!(fs::read(addon_dir(&f).join(name)).unwrap(), bytes);
    }
    no_temp_files(&addon_dir(&f));
    assert_eq!(list_flavors(t.path())[0].addon, AddonState::Installed { version: Some(addon::VERSION.into()) });
}

#[test]
fn install_updates_an_older_copy_and_keeps_other_addons() {
    let t = TempDir::new();
    let f = make_flavor(t.path(), "_forever_", "Wow.exe");
    write_installed(&f, "0.1.0");
    fs::write(addon_dir(&f).join("WoWChatTranslator.lua"), b"old").unwrap();
    let other = f.join("Interface").join("AddOns").join("Details");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("Details.toc"), b"## Version: 9\n").unwrap();

    let r = install(&f, &addon::files(), false);
    assert_eq!(r, InstallResult::Updated { from: Some("0.1.0".into()) });
    assert_eq!(fs::read(addon_dir(&f).join("WoWChatTranslator.lua")).unwrap(), embedded("WoWChatTranslator.lua"));
    assert_eq!(fs::read(other.join("Details.toc")).unwrap(), b"## Version: 9\n");
    no_temp_files(&addon_dir(&f));
}

#[test]
fn install_refuses_a_missing_flavor_folder() {
    let t = TempDir::new();
    let r = install(&t.path().join("_nope_"), &addon::files(), false);
    assert!(matches!(r, InstallResult::Failed(_)));
    assert!(!t.path().join("_nope_").exists());
}

#[cfg(windows)]
fn mklink_junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed");
}

#[cfg(windows)]
#[test]
fn junction_is_detected_and_skipped() {
    let t = TempDir::new();
    let dev = t.path().join("dev-copy");
    fs::create_dir_all(&dev).unwrap();
    fs::write(dev.join("WoWChatTranslator.toc"), b"## Version: 0.1.0\r\n").unwrap();
    fs::write(dev.join("WoWChatTranslator.lua"), b"dev lua").unwrap();
    let f = make_flavor(t.path(), "_forever_", "Wow.exe");
    fs::create_dir_all(f.join("Interface").join("AddOns")).unwrap();
    mklink_junction(&addon_dir(&f), &dev);

    let state = list_flavors(t.path())[0].addon.clone();
    assert!(matches!(&state, AddonState::Linked { version: Some(v), .. } if v == "0.1.0"), "{state:?}");

    let r = install(&f, &addon::files(), false);
    assert!(matches!(r, InstallResult::SkippedLinked { .. }), "{r:?}");
    assert_eq!(fs::read(dev.join("WoWChatTranslator.lua")).unwrap(), b"dev lua", "never writes through the link");
}

#[cfg(windows)]
#[test]
fn replacing_a_junction_removes_only_the_link() {
    let t = TempDir::new();
    let dev = t.path().join("dev-copy");
    fs::create_dir_all(&dev).unwrap();
    fs::write(dev.join("WoWChatTranslator.lua"), b"dev lua").unwrap();
    let f = make_flavor(t.path(), "_forever_", "Wow.exe");
    fs::create_dir_all(f.join("Interface").join("AddOns")).unwrap();
    mklink_junction(&addon_dir(&f), &dev);

    let r = install(&f, &addon::files(), true);
    assert_eq!(r, InstallResult::ReplacedLink);
    assert_eq!(fs::read(dev.join("WoWChatTranslator.lua")).unwrap(), b"dev lua", "link target untouched");
    assert_eq!(fs::read_dir(&dev).unwrap().count(), 1);
    let meta = fs::symlink_metadata(addon_dir(&f)).unwrap();
    assert!(meta.is_dir() && !meta.file_type().is_symlink());
    assert_eq!(fs::read(addon_dir(&f).join("WoWChatTranslator.lua")).unwrap(), embedded("WoWChatTranslator.lua"));
}

// Prefs ---------------------------------------------------------------------------------------------

#[test]
fn addon_prefs_round_trip() {
    let t = TempDir::new();
    let path = t.path().join("sub").join("addon.json");
    assert_eq!(addon_prefs::load(&path), AddonPrefs::default());
    let prefs = AddonPrefs { addon_prompt_done: true, extra_folders: vec![PathBuf::from(r"D:\Games\WoW")] };
    addon_prefs::save(&path, &prefs).unwrap();
    assert_eq!(addon_prefs::load(&path), prefs);
    assert!(fs::read_to_string(&path).unwrap().contains("\"addonPromptDone\": true"));
}

/// Read-only report of this PC's WoW installs: `cargo test --test addon -- --ignored --nocapture`.
#[cfg(windows)]
#[test]
#[ignore]
fn print_detection_on_this_pc() {
    use wow_chat_translator::addon::system::WindowsSystem;
    let d = detect(&WindowsSystem, &[]);
    println!("last played: {:?}", d.last_played);
    for root in &d.roots {
        println!("root {} {:?}", root.path.display(), root.sources);
        for f in &root.flavors {
            println!("  {} = {} -> {:?}{}", f.folder, f.name, f.addon, if d.is_last_played(f) { " (last played)" } else { "" });
        }
    }
}

#[test]
fn embedded_files_write_the_toc_last() {
    // The .toc carries the version: writing it last means a failed .lua write never
    // leaves a folder that claims the new version with the old code.
    let names: Vec<&str> = wow_chat_translator::addon::files().iter().map(|(n, _)| *n).collect();
    assert_eq!(names.last(), Some(&"WoWChatTranslator.toc"));
}

/// An embedded addon file by name (install order is not part of the contract).
fn embedded(name: &str) -> &'static [u8] {
    wow_chat_translator::addon::files().iter().find(|(n, _)| *n == name).map(|(_, b)| *b).expect(name)
}

#[test]
fn reinstalling_the_same_version_says_reinstalled_not_updated() {
    let same = InstallResult::Updated { from: Some(addon::VERSION.to_string()) };
    assert_eq!(same.message(), format!("reinstalado (ya estaba en {})", addon::VERSION));
    // An older version still reads as an update.
    let older = InstallResult::Updated { from: Some("0.1.0".into()) };
    assert_eq!(older.message(), "actualizado (antes 0.1.0)");
    assert_eq!(InstallResult::Updated { from: None }.message(), "actualizado");
}
