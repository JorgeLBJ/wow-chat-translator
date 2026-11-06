//! In-process Win32 capture of the small patch at the top-left of the WoW client area,
//! where the addon draws its pixel strip. Replaces `app/ocr/strip-worker.ps1`.
//!
//! Read-only screen capture: no memory reading, no input automation.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
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
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
    GW_OWNER,
};

pub use crate::addon::detect::is_wow_client_exe;

/// Patch size grabbed every tick, in physical pixels (same as the PowerShell worker).
pub const PATCH_W: usize = 260;
pub const PATCH_H: usize = 28;

const WOW_TITLE: PCWSTR = w!("World of Warcraft");
/// How often the process-based window search may run while no titled window exists.
const PROCESS_SEARCH_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// Result of one capture attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grab {
    /// No WoW window (titled "World of Warcraft" or owned by a running client).
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
    last_process_search: Option<std::time::Instant>,
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
        self.hwnd = 0;
        let titled = unsafe { FindWindowW(PCWSTR::null(), WOW_TITLE) }.ok().filter(|h| !h.is_invalid());
        let found = titled.or_else(|| {
            // Other clients may use another title: any visible top-level window of a
            // running Wow*.exe, searched at most once a second.
            let due = self.last_process_search.is_none_or(|t| t.elapsed() >= PROCESS_SEARCH_EVERY);
            if !due {
                return None;
            }
            self.last_process_search = Some(std::time::Instant::now());
            let pids: Vec<u32> = wow_client_processes().into_iter().map(|(pid, _)| pid).collect();
            client_window(&pids)
        })?;
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

/// Running game clients (`Wow*.exe`, see [`is_wow_client_exe`]) as (pid, exe name).
pub fn wow_client_processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if is_wow_client_exe(&name) {
                out.push((entry.th32ProcessID, name));
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// True when any game client (`Wow.exe`, `WowClassic.exe`, `WowClassicT.exe`...) runs.
pub fn is_game_running() -> bool {
    !wow_client_processes().is_empty()
}

struct WindowSearch<'a> {
    pids: &'a [u32],
    best: Option<(HWND, i64)>,
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let search = unsafe { &mut *(lparam.0 as *mut WindowSearch) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let owned = unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
    if search.pids.contains(&pid) && !owned && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if search.best.is_none_or(|(_, a)| area > a) {
                search.best = Some((hwnd, area));
            }
        }
    }
    true.into()
}

/// The largest visible, unowned top-level window of one of `pids`.
fn client_window(pids: &[u32]) -> Option<HWND> {
    if pids.is_empty() {
        return None;
    }
    let mut search = WindowSearch { pids, best: None };
    unsafe {
        let _ = EnumWindows(Some(collect_window), LPARAM(&mut search as *mut WindowSearch as isize));
    }
    search.best.map(|(h, _)| h)
}
