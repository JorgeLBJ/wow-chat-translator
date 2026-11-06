//! Starts the app: single instance, config, translator, window, tray and threads.

use std::sync::mpsc;
use std::sync::Arc;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::capture;
use crate::core::prefs;
use crate::translators::build_translator;

use super::debug_log::{DebugLog, DEBUG_MAX_BYTES};
use super::shared::{Shared, StatusInfo};
use super::style::{self, translator_label, ReaderState};
use super::ui::OverlayApp;
use super::win::{self, Hwnd};
use super::{hotkey, instance, monitor, tray, workers};

const ICON_256: &[u8] = include_bytes!("../../assets/icon-256.png");
const ICON_32: &[u8] = include_bytes!("../../assets/icon-32.png");
const TITLE: &str = "WoW Chat Translator";
const DEFAULT_W: f32 = 560.0;
const DEFAULT_H: f32 = 420.0;

pub fn run() -> eframe::Result {
    capture::init_dpi_awareness();
    let Some(instance) = instance::acquire() else {
        return Ok(()); // already running: it was asked to show its window
    };
    // Started with Windows: stay in the tray until the game shows the strip.
    let started_hidden = std::env::args().any(|a| a == "--hidden");

    let dir = win::data_dir();
    let config_path = dir.join("config.json");
    let config = prefs::load(&config_path);
    let log = DebugLog::new(dir.join("debug.log"), DEBUG_MAX_BYTES);
    log.write(&format!("app start hidden={started_hidden}"));

    let built = build_translator(&config);
    if let Some(issue) = &built.issue {
        log.write(&format!("translator issue: {issue}"));
    }
    let status = StatusInfo {
        reader: ReaderState::Idle,
        text: built.issue.clone().unwrap_or_default(),
        translator: translator_label(&config.translator),
        transient: None,
    };
    let (job_tx, job_rx) = mpsc::channel();

    let viewport = viewport(&config);
    let shared = Arc::new(Shared::new(
        config,
        config_path,
        log,
        Arc::from(built.translator),
        status,
        job_tx,
        Some(instance),
    ));

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            setup_style(&cc.egui_ctx);
            shared.set_context(cc.egui_ctx.clone());
            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::Win32(h) = handle.as_raw() {
                    shared.set_hwnd(Hwnd(h.hwnd.get()));
                }
            }
            let hwnd = shared.hwnd();
            hwnd.exclude_from_capture();
            // --hidden: invisible until revealed (eframe shows the window after frame 1).
            hwnd.set_opacity(if started_hidden { 0.0 } else { shared.config().prefs.opacity });
            if let Err(e) = tray::create(shared.clone(), ICON_32) {
                shared.log.write(&format!("tray failed: {e}"));
            }
            workers::spawn(shared.clone(), job_rx);
            monitor::spawn(shared.clone());
            hotkey::spawn(shared.clone());
            let icon = eframe::icon_data::from_png_bytes(ICON_256).ok().map(Arc::new);
            Ok(Box::new(OverlayApp::new(shared, started_hidden, icon)))
        }),
    )
}

fn viewport(config: &prefs::Config) -> egui::ViewportBuilder {
    let b = config.overlay_bounds.unwrap_or_default();
    let (wx, wy, ww, _) = win::work_area_points();
    let width = b.width.map_or(DEFAULT_W, |w| w.max(320) as f32);
    let height = b.height.map_or(DEFAULT_H, |h| h.max(180) as f32);
    let x = b.x.map_or(wx + ww - DEFAULT_W - 20.0, |x| x as f32);
    let y = b.y.map_or(wy + 20.0, |y| y as f32);
    let mut builder = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_decorations(false)
        .with_resizable(true)
        .with_inner_size([width, height])
        .with_min_inner_size([320.0, 180.0])
        .with_position([x, y]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_256) {
        builder = builder.with_icon(icon);
    }
    if config.prefs.always_on_top {
        builder = builder.with_always_on_top();
    }
    builder
}

fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = style::BG;
    visuals.window_fill = style::BG;
    visuals.override_text_color = None;
    visuals.widgets.noninteractive.fg_stroke.color = style::TEXT;
    visuals.selection.bg_fill = style::ACCENT;
    visuals.slider_trailing_fill = true;
    ctx.set_visuals(visuals);
    ctx.style_mut(|s| {
        s.interaction.selectable_labels = false;
        s.spacing.slider_width = 80.0;
    });
}
