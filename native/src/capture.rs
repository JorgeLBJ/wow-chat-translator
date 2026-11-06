//! In-process Win32 capture of the small patch at the top-left of the WoW client area,
//! where the addon draws its pixel strip. Replaces `app/ocr/strip-worker.ps1`.
//!
//! Read-only screen capture: no memory reading, no input automation.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, IsWindow};

/// Patch size grabbed every tick, in physical pixels (same as the PowerShell worker).
pub const PATCH_W: usize = 260;
pub const PATCH_H: usize = 28;

const WOW_TITLE: PCWSTR = w!("World of Warcraft");
const WOW_EXE: &str = "wow.exe";

/// Result of one capture attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grab {
    /// No window titled exactly "World of Warcraft".
    WowNotFound,
    /// The WoW window exists but is minimized.
    Minimized,
    /// Tightly packed RGB bytes, `PATCH_W * PATCH_H * 3`.
    Patch(Vec<u8>),
}

/// Makes the process per-monitor DPI aware so window coordinates are physical pixels;
/// without this, Windows scaling would shift the capture. Safe to call more than once
/// (later calls fail harmlessly once the awareness is set).
pub fn init_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Captures the WoW client-area patch. Keeps the window handle between calls.
#[derive(Debug, Default)]
pub struct Capturer {
    // Raw HWND value; kept as an integer so the capturer is `Send`.
    hwnd: isize,
}

impl Capturer {
    pub fn new() -> Self {
        Self::default()
    }

    fn window(&mut self) -> Option<HWND> {
        let cached = HWND(self.hwnd as *mut _);
        if self.hwnd != 0 && unsafe { IsWindow(Some(cached)) }.as_bool() {
            return Some(cached);
        }
        let found = unsafe { FindWindowW(PCWSTR::null(), WOW_TITLE) }.ok()?;
        if found.is_invalid() {
            self.hwnd = 0;
            return None;
        }
        self.hwnd = found.0 as isize;
        Some(found)
    }

    /// Grabs a `PATCH_W` x `PATCH_H` patch at the WoW client-area origin.
    pub fn grab(&mut self) -> Result<Grab, String> {
        let Some(hwnd) = self.window() else {
            return Ok(Grab::WowNotFound);
        };
        if unsafe { IsIconic(hwnd) }.as_bool() {
            return Ok(Grab::Minimized);
        }
        let mut origin = POINT { x: 0, y: 0 };
        if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
            self.hwnd = 0;
            return Ok(Grab::WowNotFound);
        }
        grab_screen(origin.x, origin.y, PATCH_W, PATCH_H).map(Grab::Patch)
    }
}

/// BitBlts a screen rectangle into a 32-bit top-down DIB and returns it as RGB bytes.
fn grab_screen(x: i32, y: i32, w: usize, h: usize) -> Result<Vec<u8>, String> {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC failed".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // negative = top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let result = match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(dib) => {
                let old = SelectObject(mem, dib.into());
                let blt = BitBlt(mem, 0, 0, w as i32, h as i32, Some(screen), x, y, SRCCOPY);
                let out = match blt {
                    Ok(()) if !bits.is_null() => {
                        // BGRA, stride = w * 4 (32-bit rows are always DWORD aligned).
                        let bgra = std::slice::from_raw_parts(bits as *const u8, w * h * 4);
                        let mut rgb = Vec::with_capacity(w * h * 3);
                        for p in bgra.chunks_exact(4) {
                            rgb.extend_from_slice(&[p[2], p[1], p[0]]);
                        }
                        Ok(rgb)
                    }
                    Ok(()) => Err("CreateDIBSection returned no bits".to_string()),
                    Err(e) => Err(format!("BitBlt failed: {e}")),
                };
                SelectObject(mem, old);
                let _ = DeleteObject(dib.into());
                out
            }
            Err(e) => Err(format!("CreateDIBSection failed: {e}")),
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        result
    }
}

/// True when a process named `Wow.exe` (case-insensitive) is running.
pub fn is_game_running() -> bool {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(WOW_EXE) {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}
