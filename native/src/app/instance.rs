//! Single instance: a named mutex marks the running app; a second launch signals a
//! named event instead of opening another window, and the first instance shows itself.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
};

const MUTEX: PCWSTR = w!("Local\\WoWChatTranslatorNative");
const SHOW_EVENT: PCWSTR = w!("Local\\WoWChatTranslatorNative.Show");

/// Held for the life of the first instance.
pub struct InstanceGuard {
    // Raw handles as integers so the guard is Send + Sync.
    mutex: isize,
    event: isize,
}

impl InstanceGuard {
    /// True (once per request) when a second launch asked to show the window.
    pub fn show_requested(&self) -> bool {
        self.event != 0 && unsafe { WaitForSingleObject(HANDLE(self.event as *mut _), 0) } == WAIT_OBJECT_0
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            for h in [self.mutex, self.event] {
                if h != 0 {
                    let _ = CloseHandle(HANDLE(h as *mut _));
                }
            }
        }
    }
}

/// `Some` for the first instance. `None` when another one runs: it has been asked to
/// show its window and this process should exit.
pub fn acquire() -> Option<InstanceGuard> {
    unsafe {
        let mutex = CreateMutexW(None, false, MUTEX).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, SHOW_EVENT) {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
            return None;
        }
        // Auto-reset: one wait consumes one request.
        let event = CreateEventW(None, false, false, SHOW_EVENT).map(|h| h.0 as isize).unwrap_or(0);
        Some(InstanceGuard { mutex: mutex.0 as isize, event })
    }
}
