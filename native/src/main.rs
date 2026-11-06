// No console window in release builds (the app lives in the tray).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> eframe::Result {
    wow_chat_translator::app::run::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("WoW Chat Translator only runs on Windows");
}
