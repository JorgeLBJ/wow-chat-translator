//! The overlay app: window, tray, background reader and translation workers.
//! Pure logic (testable without a window) lives in `presence`, `game_watcher`,
//! `messages`, `style` and `debug_log`; the rest is Windows-only glue.

pub mod compose;
pub mod debug_log;
pub mod game_watcher;
pub mod messages;
pub mod presence;
pub mod settings_form;
pub mod style;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod hotkey;
#[cfg(windows)]
pub mod instance;
#[cfg(windows)]
pub mod monitor;
#[cfg(windows)]
pub mod run;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod shared;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod ui;
#[cfg(windows)]
pub mod win;
#[cfg(windows)]
pub mod workers;
