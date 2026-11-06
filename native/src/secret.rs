//! The API key at rest: encrypted with Windows DPAPI for the current user and stored
//! as base64 in `translator.apiKeyEnc`, bound to the provider + server it was saved
//! for (`translator.keyBinding`). Keys saved by Electron's safeStorage are not DPAPI
//! blobs, so they read as no key and must be entered again.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::core::prefs::Config;
use crate::translators::effective_url;
use crate::translators::key_binding::key_binding;

/// DPAPI-encrypts the key and returns it as base64.
pub fn encrypt_api_key(plain: &str) -> Result<String, String> {
    Ok(STANDARD.encode(dpapi::protect(plain.as_bytes())?))
}

/// Reverses [`encrypt_api_key`]; fails for anything not encrypted by this user.
pub fn decrypt_api_key(enc: &str) -> Result<String, String> {
    let blob = STANDARD.decode(enc.trim()).map_err(|e| e.to_string())?;
    let plain = dpapi::unprotect(&blob)?;
    String::from_utf8(plain).map_err(|_| "la clave guardada no es texto".to_string())
}

/// The saved key, only when it was saved for this same provider + server, so it can
/// never be sent to a different one. Missing, mismatched or undecryptable: `None`.
pub fn read_api_key(config: &Config, provider: &str, base_url: &str) -> Option<String> {
    let t = &config.translator;
    let enc = t.api_key_enc.as_deref().filter(|e| !e.is_empty())?;
    if t.key_binding.as_deref() != Some(key_binding(provider, &effective_url(provider, base_url)).as_str()) {
        return None;
    }
    decrypt_api_key(enc).ok().filter(|k| !k.is_empty())
}

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if blob.pbData.is_null() {
            return Vec::new();
        }
        // SAFETY: DPAPI returned `cbData` bytes at `pbData`, allocated with LocalAlloc.
        let out = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(Some(HLOCAL(blob.pbData as _)));
        }
        out
    }

    fn input(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: input points at `data` for the call; DPAPI does not write to it.
        unsafe { CryptProtectData(&input(data), None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|e| e.message())?;
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`.
        unsafe { CryptUnprotectData(&input(data), None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|e| e.message())?;
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("DPAPI solo existe en Windows".into())
    }
    pub fn unprotect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("DPAPI solo existe en Windows".into())
    }
}
