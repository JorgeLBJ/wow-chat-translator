//! Tray icon: click shows the window; menu "Mostrar ventana", "Iniciar con Windows"
//! (checkbox) and "Salir" (the only way to quit: ✕ only hides).
//!
//! Created on the UI thread (it owns the event loop). Menu and click handlers run on
//! that same thread, so the icon itself can live in a thread-local and be removed
//! before exiting.

use std::cell::RefCell;
use std::sync::Arc;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::autostart;
use super::shared::Shared;

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    static AUTO_ITEM: RefCell<Option<CheckMenuItem>> = const { RefCell::new(None) };
}

/// Keeps the tray checkbox in step when the settings window changes "Iniciar con
/// Windows" (both run on the UI thread).
pub fn set_autostart_checked(checked: bool) {
    AUTO_ITEM.with(|a| {
        if let Some(item) = a.borrow().as_ref() {
            item.set_checked(checked);
        }
    });
}

pub fn create(shared: Arc<Shared>, icon_png: &[u8]) -> Result<(), String> {
    let icon = eframe::icon_data::from_png_bytes(icon_png).map_err(|e| e.to_string())?;
    let icon = Icon::from_rgba(icon.rgba, icon.width, icon.height).map_err(|e| e.to_string())?;

    let show = MenuItem::new("Mostrar ventana", true, None);
    let auto = CheckMenuItem::new("Iniciar con Windows", true, autostart::is_enabled(), None);
    let quit = MenuItem::new("Salir", true, None);
    let menu = Menu::new();
    menu.append_items(&[&show, &PredefinedMenuItem::separator(), &auto, &PredefinedMenuItem::separator(), &quit])
        .map_err(|e| e.to_string())?;

    let tray = TrayIconBuilder::new()
        .with_tooltip("WoW Chat Translator")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())?;
    TRAY.with(|t| *t.borrow_mut() = Some(tray));
    AUTO_ITEM.with(|a| *a.borrow_mut() = Some(auto.clone()));

    let (show_id, auto_id, quit_id) = (show.id().clone(), auto.id().clone(), quit.id().clone());
    let s = shared.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            s.show_by_user();
        } else if event.id == auto_id {
            // The checkbox has already flipped; follow the real registry state.
            let enable = !autostart::is_enabled();
            match autostart::set_enabled(enable) {
                Ok(()) => {
                    s.config().prefs.auto_start = enable;
                    s.save_config();
                    s.log.write(&format!("autostart {enable}"));
                }
                Err(e) => s.log.write(&format!("autostart failed: {e}")),
            }
        } else if event.id == quit_id {
            quit_app(&s);
        }
    }));

    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            shared.show_by_user();
        }
    }));
    Ok(())
}

/// Saves, removes the tray icon and ends the process (worker threads included).
pub fn quit_app(shared: &Shared) {
    shared.log.write("quit");
    shared.save_config();
    TRAY.with(|t| t.borrow_mut().take());
    std::process::exit(0);
}
