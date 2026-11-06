//! Installs or updates the bundled addon into one flavor:
//! `<flavor>\Interface\AddOns\WoWChatTranslator\`. Each file is written to a temp file
//! next to it and renamed over the old one, so WoW never reads half a file. Only this
//! addon's folder is touched. A junction or symlink there (a developer's working copy)
//! is never written through: it is skipped, or with `replace_link` only the link
//! itself is removed and a real folder takes its place.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::detect::{addon_dir, addon_state, AddonState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallResult {
    Installed,
    Updated { from: Option<String> },
    /// The addon folder is a link to a development copy: left alone.
    SkippedLinked { target: Option<PathBuf> },
    /// The link was removed (its target untouched) and the addon installed.
    ReplacedLink,
    Failed(String),
}

impl InstallResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Installed | Self::Updated { .. } | Self::ReplacedLink)
    }

    /// Short Spanish description for the UI.
    pub fn message(&self) -> String {
        match self {
            Self::Installed => "instalado".into(),
            Self::Updated { from: Some(v) } => format!("actualizado (antes {v})"),
            Self::Updated { from: None } => "actualizado".into(),
            Self::SkippedLinked { .. } => "enlazado a una copia de desarrollo: no se tocó".into(),
            Self::ReplacedLink => "enlace reemplazado e instalado".into(),
            Self::Failed(e) => format!("error: {e}"),
        }
    }
}

fn describe(e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::PermissionDenied {
        "sin permiso para escribir en la carpeta de WoW".into()
    } else {
        e.to_string()
    }
}

/// Writes `bytes` to `dir\name` through `dir\.name.wct-tmp` + rename.
fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join(format!(".{name}.wct-tmp"));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, dir.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Removes the link at `path`, never its target's contents.
fn remove_link(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if fs::symlink_metadata(path)?.file_type().is_symlink_dir() {
            // RemoveDirectoryW on a junction / directory symlink deletes only the link.
            return fs::remove_dir(path);
        }
    }
    fs::remove_file(path)
}

/// Installs `files` (name, bytes) into `flavor`. The flavor folder must exist;
/// `Interface\AddOns\WoWChatTranslator` is created as needed.
pub fn install(flavor: &Path, files: &[(&str, &[u8])], replace_link: bool) -> InstallResult {
    if !flavor.is_dir() {
        return InstallResult::Failed(format!("no existe la carpeta {}", flavor.display()));
    }
    let dir = addon_dir(flavor);
    let before = addon_state(flavor);
    let mut replaced = false;
    if let AddonState::Linked { target, .. } = &before {
        if !replace_link {
            return InstallResult::SkippedLinked { target: target.clone() };
        }
        if let Err(e) = remove_link(&dir) {
            return InstallResult::Failed(describe(&e));
        }
        replaced = true;
    }
    if let Err(e) = fs::create_dir_all(&dir) {
        return InstallResult::Failed(describe(&e));
    }
    for (name, bytes) in files {
        if let Err(e) = write_atomic(&dir, name, bytes) {
            return InstallResult::Failed(describe(&e));
        }
    }
    match (replaced, before) {
        (true, _) => InstallResult::ReplacedLink,
        (false, AddonState::Installed { version }) => InstallResult::Updated { from: version },
        _ => InstallResult::Installed,
    }
}

/// Installs the addon bundled in the app.
pub fn install_bundled(flavor: &Path, replace_link: bool) -> InstallResult {
    install(flavor, &super::files(), replace_link)
}
