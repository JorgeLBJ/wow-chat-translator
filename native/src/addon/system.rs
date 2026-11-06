//! Windows side of detection: the registry, running game paths and the folder picker.
//! Read-only except for the dialog the user drives.

use std::path::PathBuf;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

use super::detect::{UninstallEntry, WowSystem};
use crate::capture;

const BLIZZARD_KEY: PCWSTR = w!(r"SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft");
const UNINSTALL_KEYS: [PCWSTR; 2] = [
    w!(r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
];

/// The real registry and process list.
pub struct WindowsSystem;

fn reg_string(key: HKEY, subkey: PCWSTR, value: &str) -> Option<String> {
    let value = HSTRING::from(value);
    let mut size: u32 = 0;
    unsafe {
        if RegGetValueW(key, subkey, &value, RRF_RT_REG_SZ, None, None, Some(&mut size)) != ERROR_SUCCESS || size < 2 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        if RegGetValueW(key, subkey, &value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

fn uninstall_entries_under(path: PCWSTR, out: &mut Vec<UninstallEntry>) {
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path, None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return;
        }
        let mut index = 0;
        loop {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(key, index, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) != ERROR_SUCCESS {
                break;
            }
            index += 1;
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            let sub_h = HSTRING::from(sub.as_str());
            let sub_p = PCWSTR(sub_h.as_ptr());
            out.push(UninstallEntry {
                display_name: reg_string(key, sub_p, "DisplayName"),
                install_location: reg_string(key, sub_p, "InstallLocation"),
                key: sub,
            });
        }
        let _ = RegCloseKey(key);
    }
}

fn process_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(handle);
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

impl WowSystem for WindowsSystem {
    fn install_path(&self) -> Option<String> {
        reg_string(HKEY_LOCAL_MACHINE, BLIZZARD_KEY, "InstallPath")
    }

    fn uninstall_entries(&self) -> Vec<UninstallEntry> {
        let mut out = Vec::new();
        for path in UNINSTALL_KEYS {
            uninstall_entries_under(path, &mut out);
        }
        out
    }

    fn running_wow_exes(&self) -> Vec<PathBuf> {
        capture::wow_client_processes().into_iter().filter_map(|(pid, _)| process_path(pid)).collect()
    }
}

/// The native "choose a folder" dialog. Blocks the calling thread (run it on its own
/// thread); `owner` keeps the dialog above that window. `None` when cancelled.
pub fn pick_folder(owner: Option<isize>, title: &str) -> Option<PathBuf> {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let result = (|| -> windows::core::Result<PathBuf> {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
            dialog.SetTitle(&HSTRING::from(title))?;
            dialog.Show(owner.map(|h| HWND(h as *mut _)))?;
            let item = dialog.GetResult()?;
            let raw = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = PathBuf::from(raw.to_string().unwrap_or_default());
            CoTaskMemFree(Some(raw.0 as *const _));
            Ok(path)
        })();
        if com {
            CoUninitialize();
        }
        result.ok().filter(|p| !p.as_os_str().is_empty())
    }
}
