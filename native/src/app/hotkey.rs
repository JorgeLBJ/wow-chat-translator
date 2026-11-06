//! The only global hotkey, Ctrl+Shift+Enter (quick compose). It is registered only
//! while the game runs, so it never takes the combination from other programs.
//! A small thread owns the registration (hotkeys belong to the registering thread)
//! and polls its message queue for `WM_HOTKEY`.

use std::sync::Arc;
use std::time::Duration;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE, WM_HOTKEY};

use super::shared::Shared;

const HOTKEY_ID: i32 = 0x5743; // "WC"
const POLL: Duration = Duration::from_millis(100);

pub fn spawn(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("hotkey".into()).spawn(move || run(shared));
}

fn run(shared: Arc<Shared>) {
    let mut registered = false;
    // A failed registration (another program owns it) is retried only next game session.
    let mut failed = false;
    loop {
        let want = shared.game_running();
        if want && !registered && !failed {
            let ok = unsafe { RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, VK_RETURN.0 as u32) };
            match ok {
                Ok(()) => registered = true,
                Err(e) => {
                    failed = true;
                    shared.log.write(&format!("hotkey Ctrl+Shift+Enter not registered: {}", e.message()));
                }
            }
        } else if !want {
            if registered {
                unsafe {
                    let _ = UnregisterHotKey(None, HOTKEY_ID);
                }
                registered = false;
            }
            failed = false;
        }
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_ID as usize && registered {
                shared.request_compose_focus();
            }
        }
        std::thread::sleep(POLL);
    }
}
