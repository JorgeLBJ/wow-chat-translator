//! Win32 helpers for the overlay window. Visibility is driven with `ShowWindowAsync`
//! because it is called from background threads and while the UI thread may be busy;
//! eframe does not run `update` while the window is hidden, so the window cannot be
//! shown through egui viewport commands.

use std::path::PathBuf;

use windows::Win32::Foundation::{FILETIME, HWND, RECT, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, IsWindowVisible, SetForegroundWindow, SetLayeredWindowAttributes,
    SetWindowDisplayAffinity, SetWindowLongPtrW, ShowWindowAsync, SystemParametersInfoW, GWL_EXSTYLE, LWA_ALPHA,
    SPI_GETWORKAREA, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    WDA_EXCLUDEFROMCAPTURE, WS_EX_LAYERED,
};

/// A window handle that can cross threads (stored as an integer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hwnd(pub isize);

impl Hwnd {
    fn get(self) -> Option<HWND> {
        (self.0 != 0).then_some(HWND(self.0 as *mut _))
    }

    pub fn is_set(self) -> bool {
        self.0 != 0
    }

    /// Shows the window without taking the focus (the game keeps it).
    pub fn show_inactive(self) {
        if let Some(h) = self.get() {
            unsafe {
                let _ = ShowWindowAsync(h, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// Shows and activates the window (tray "Mostrar ventana", second launch).
    pub fn show_active(self) {
        if let Some(h) = self.get() {
            unsafe {
                let _ = ShowWindowAsync(h, SW_SHOW);
                let _ = SetForegroundWindow(h);
            }
        }
    }

    pub fn hide(self) {
        if let Some(h) = self.get() {
            unsafe {
                let _ = ShowWindowAsync(h, SW_HIDE);
            }
        }
    }

    pub fn is_visible(self) -> bool {
        self.get().is_some_and(|h| unsafe { IsWindowVisible(h) }.as_bool())
    }

    /// True when the window is the foreground window (it has the keyboard focus).
    pub fn is_foreground(self) -> bool {
        self.get().is_some_and(|h| unsafe { GetForegroundWindow() } == h)
    }

    /// Whole-window opacity (0..=1). Below 1 the window becomes layered; at 1 it is a
    /// plain window again (no compositing cost).
    pub fn set_opacity(self, opacity: f32) {
        let Some(h) = self.get() else { return };
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            let layered = WS_EX_LAYERED.0 as isize;
            if opacity >= 0.999 {
                if ex & layered != 0 {
                    SetWindowLongPtrW(h, GWL_EXSTYLE, ex & !layered);
                }
                return;
            }
            if ex & layered == 0 {
                SetWindowLongPtrW(h, GWL_EXSTYLE, ex | layered);
            }
            let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
            let _ = SetLayeredWindowAttributes(h, Default::default(), alpha, LWA_ALPHA);
        }
    }

    /// Keeps the window out of screen captures, so the strip reader still sees the
    /// addon strip when the overlay sits on top of it (Electron's setContentProtection).
    pub fn exclude_from_capture(self) {
        if let Some(h) = self.get() {
            unsafe {
                let _ = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
            }
        }
    }
}

/// Minutes to add to UTC to get local time at `epoch_ms` (DST-aware).
pub fn utc_offset_minutes_at(epoch_ms: u64) -> i32 {
    // FILETIME: 100 ns ticks since 1601-01-01.
    let ticks = (epoch_ms + 11_644_473_600_000) * 10_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    unsafe {
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        let mut local_ft = FILETIME::default();
        if FileTimeToSystemTime(&ft, &mut utc).is_err()
            || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err()
            || SystemTimeToFileTime(&local, &mut local_ft).is_err()
        {
            return 0;
        }
        let local_ticks = ((local_ft.dwHighDateTime as u64) << 32) | local_ft.dwLowDateTime as u64;
        ((local_ticks as i64 - ticks as i64) / 600_000_000) as i32
    }
}

/// Primary work area in logical points (system DPI): x, y, width, height.
pub fn work_area_points() -> (f32, f32, f32, f32) {
    unsafe {
        let mut r = RECT::default();
        let ok = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut RECT as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        if !ok {
            return (0.0, 0.0, 1280.0, 720.0);
        }
        let scale = GetDpiForSystem().max(96) as f32 / 96.0;
        (
            r.left as f32 / scale,
            r.top as f32 / scale,
            (r.right - r.left) as f32 / scale,
            (r.bottom - r.top) as f32 / scale,
        )
    }
}

/// `%APPDATA%\wow-chat-translator-native` (separate from Electron's folder).
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("wow-chat-translator-native")
}

/// Copies `text` to the clipboard as Unicode text (the overlay window owns it).
pub fn copy_to_clipboard(owner: Hwnd, text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_UNICODETEXT: u32 = 13;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // Another program may hold the clipboard for a moment.
    let mut opened = false;
    for _ in 0..10 {
        if unsafe { OpenClipboard(owner.get()) }.is_ok() {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !opened {
        return Err("el portapapeles está ocupado".into());
    }
    let result = unsafe {
        (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.message())?;
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(|e| e.message())?;
            let ptr = GlobalLock(mem) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(mem));
                return Err("sin memoria para el portapapeles".into());
            }
            // SAFETY: `mem` holds `wide.len()` u16 values and is locked.
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(mem);
            // On success the clipboard owns the memory.
            if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))) {
                let _ = GlobalFree(Some(mem));
                return Err(e.message());
            }
            Ok(())
        })()
    };
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

/// Brings the WoW window to the front (restoring it if minimized). It only changes the
/// active window and never sends keystrokes. Windows allows it while this app has the
/// focus (the user just pressed Enter or Esc in the compose box). True when WoW is now
/// the foreground window.
pub fn focus_game() -> bool {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, ShowWindow, SW_RESTORE};
    unsafe {
        let Ok(game) = FindWindowW(None, w!("World of Warcraft")) else { return false };
        if game.is_invalid() {
            return false;
        }
        if IsIconic(game).as_bool() {
            let _ = ShowWindow(game, SW_RESTORE);
        }
        let _ = SetForegroundWindow(game);
        GetForegroundWindow() == game
    }
}
