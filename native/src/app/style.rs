//! Colors, labels and small formatters shared by the overlay (same tables as
//! `app/src/renderer/overlay.js` / `overlay.html`).

use eframe::egui::Color32;

use crate::core::prefs::TranslatorSettings;
use crate::translators::provider_label;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const BG: Color32 = hex(0x0e1016);
pub const BORDER: Color32 = hex(0x262a31);
pub const TEXT: Color32 = hex(0xe6e6e6);
pub const MUTED: Color32 = hex(0x7d8590);
pub const DIM: Color32 = hex(0x5c6570);
pub const TAB_IDLE: Color32 = hex(0x8b949e);
pub const ICON: Color32 = hex(0x9aa3ad);
pub const ACCENT: Color32 = hex(0x3b82f6);
pub const ACCENT_LIGHT: Color32 = hex(0x60a5fa);
pub const AMBER: Color32 = hex(0xfbbf24);
pub const AMBER_LIGHT: Color32 = hex(0xfcd34d);
pub const AMBER_BG: Color32 = Color32::from_rgba_premultiplied(61, 40, 3, 64); // rgba(245,158,11,.25)
pub const ERROR_BG: Color32 = hex(0x7f1d1d);
pub const ERROR_TEXT: Color32 = hex(0xfecaca);
pub const CLOSE_HOVER: Color32 = hex(0xc42b1c);
pub const SEG_TEXT: Color32 = hex(0xaaaabb);

const DEFAULT_NAME: Color32 = hex(0xd0d4da);
const DEFAULT_TAB: Color32 = hex(0x9aa3ad);

/// Player name color by WoW class token.
pub fn class_color(class: Option<&str>) -> Color32 {
    match class.unwrap_or("") {
        "WARRIOR" => hex(0xC69B6D),
        "PALADIN" => hex(0xF48CBA),
        "HUNTER" => hex(0xAAD372),
        "ROGUE" => hex(0xFFF468),
        "PRIEST" => hex(0xFFFFFF),
        "SHAMAN" => hex(0x0070DD),
        "MAGE" => hex(0x3FC7EB),
        "WARLOCK" => hex(0x8788EE),
        "DRUID" => hex(0xFF7C0A),
        "DEATHKNIGHT" => hex(0xC41E3A),
        "MONK" => hex(0x00FF98),
        "DEMONHUNTER" => hex(0xA330C9),
        "EVOKER" => hex(0x33937F),
        _ => DEFAULT_NAME,
    }
}

/// Channel tag color by overlay tab key.
pub fn tab_color(tab: Option<&str>) -> Color32 {
    match tab.unwrap_or("") {
        "general" => hex(0xc8a165),
        "trade" => hex(0xd38b8b),
        "guild" => hex(0x40c040),
        "group" => hex(0x8f9cff),
        "whisper" => hex(0xff80ff),
        _ => DEFAULT_TAB,
    }
}

/// Spanish label of a channel; unknown channels keep their own name.
pub fn channel_label(channel: &str) -> String {
    let label = match channel.to_lowercase().as_str() {
        "general" => "General",
        "trade" | "comercio" => "Comercio",
        "guild" => "Hermandad",
        "officer" => "Oficial",
        "party" => "Grupo",
        "raid" => "Banda",
        "instance" => "Instancia",
        "whisper" => "Susurro",
        "say" => "Decir",
        "yell" => "Gritar",
        _ => return channel.to_string(),
    };
    label.to_string()
}

/// "HH:MM" of a time in ms since the epoch, shifted by the local UTC offset (minutes).
pub fn hhmm(epoch_ms: u64, offset_minutes: i32) -> String {
    let minutes = (epoch_ms / 60_000) as i64 + offset_minutes as i64;
    let of_day = minutes.rem_euclid(24 * 60);
    format!("{:02}:{:02}", of_day / 60, of_day % 60)
}

/// What the strip reader is doing (the dot, the footer label and the title subtitle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderState {
    Starting,
    Ok,
    Idle,
    NoStrip,
    Error,
}

impl ReaderState {
    pub fn title(self) -> &'static str {
        match self {
            ReaderState::Ok => "● leyendo",
            ReaderState::Error => "● detenido",
            ReaderState::Idle => "esperando a WoW",
            ReaderState::NoStrip => "sin franja",
            ReaderState::Starting => "iniciando…",
        }
    }

    pub fn footer(self) -> &'static str {
        match self {
            ReaderState::Ok => "Lector activo",
            ReaderState::Starting => "Lector iniciando",
            ReaderState::Error => "Lector detenido",
            ReaderState::Idle => "Esperando a WoW",
            ReaderState::NoStrip => "Sin franja del addon",
        }
    }

    pub fn dot(self) -> Color32 {
        match self {
            ReaderState::Ok => hex(0x22c55e),
            ReaderState::Idle => hex(0x6b7280),
            ReaderState::Error => hex(0xef4444),
            ReaderState::Starting | ReaderState::NoStrip => hex(0xf59e0b),
        }
    }
}

/// Footer label of the configured translator (`providerLabel` in main.js).
pub fn translator_label(settings: &TranslatorSettings) -> String {
    if settings.provider == "google" || settings.model.is_empty() {
        "Google".to_string()
    } else {
        format!("{} · {}", provider_label(&settings.provider), settings.model)
    }
}
