//! "Iniciar con Windows": the `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! value "WoW Chat Translator" = `"<exe>" --hidden` (same value name as Electron).

use std::path::Path;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("WoW Chat Translator");
/// `HKCU\Software\WoW Chat Translator` value `AutostartOptOut` = 1 while the user has
/// turned "Iniciar con Windows" off. The installer reads it: it turns autostart on for
/// new installs and upgrades unless this is set (installer/wow-chat-translator.iss).
const APP_KEY: PCWSTR = w!("Software\\WoW Chat Translator");
const OPT_OUT: PCWSTR = w!("AutostartOptOut");

/// The command registered to run at login.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" --hidden", exe.display())
}

fn current_command() -> Option<String> {
    std::env::current_exe().ok().map(|p| command_line(&p))
}

fn open() -> Option<HKEY> {
    let mut key = HKEY::default();
    let err = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_QUERY_VALUE | KEY_SET_VALUE, &mut key) };
    (err == ERROR_SUCCESS).then_some(key)
}

/// True when the Run value starts this executable.
pub fn is_enabled() -> bool {
    let (Some(key), Some(expected)) = (open(), current_command()) else { return false };
    let mut ty = REG_VALUE_TYPE::default();
    let mut buf = vec![0u8; 2048];
    let mut len = buf.len() as u32;
    let err = unsafe { RegQueryValueExW(key, VALUE, None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut len)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if err != ERROR_SUCCESS || ty != REG_SZ {
        return false;
    }
    let wide: Vec<u16> = buf[..len as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let value = String::from_utf16_lossy(&wide);
    value.trim_end_matches('\0').eq_ignore_ascii_case(&expected)
}

/// Adds or removes the Run value.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let key = open().ok_or("no se pudo abrir la clave Run")?;
    let err = if enabled {
        let cmd = current_command().ok_or("ruta del ejecutable desconocida")?;
        let wide: Vec<u16> = cmd.encode_utf16().chain([0]).collect();
        let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
        unsafe { RegSetValueExW(key, VALUE, None, REG_SZ, Some(&bytes)) }
    } else {
        let e = unsafe { RegDeleteValueW(key, VALUE) };
        if e == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
            ERROR_SUCCESS
        } else {
            e
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if err == ERROR_SUCCESS {
        remember_opt_out(!enabled);
        Ok(())
    } else {
        Err(format!("registro: error {}", err.0))
    }
}

/// Records (or clears) that the user turned autostart off, so an upgrade keeps it off.
fn remember_opt_out(opted_out: bool) {
    use windows::Win32::System::Registry::{RegCreateKeyExW, REG_DWORD, REG_OPTION_NON_VOLATILE};
    let mut key = HKEY::default();
    let err = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, APP_KEY, None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None)
    };
    if err != ERROR_SUCCESS {
        return;
    }
    unsafe {
        if opted_out {
            let _ = RegSetValueExW(key, OPT_OUT, None, REG_DWORD, Some(&1u32.to_le_bytes()));
        } else {
            let _ = RegDeleteValueW(key, OPT_OUT);
        }
        let _ = RegCloseKey(key);
    }
}
