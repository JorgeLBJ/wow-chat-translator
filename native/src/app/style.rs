//! Colors, labels and small formatters shared by the overlay (same tables as
//! `app/src/renderer/overlay.js` / `overlay.html`).

use eframe::egui::Color32;

use crate::core::item_tips::TipKind;
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
/// Item links whose quality is not known yet (WoW's link gold).
pub const LINK_GOLD: Color32 = hex(0xffd100);

/// WoW item quality colors (0 poor ... 5 legendary, 6 artifact, 7-8 heirloom/account).
pub fn item_quality_color(quality: Option<u8>) -> Color32 {
    match quality {
        Some(0) => hex(0x9d9d9d),
        Some(1) => hex(0xffffff),
        Some(2) => hex(0x1eff00),
        Some(3) => hex(0x0070dd),
        Some(4) => hex(0xa335ee),
        Some(5) => hex(0xff8000),
        Some(6) => hex(0xe6cc80),
        Some(7 | 8) => hex(0x00ccff),
        _ => LINK_GOLD,
    }
}

/// A hover-panel link's color: items in their quality color, quests in link gold.
pub fn link_color(kind: TipKind, quality: Option<u8>) -> Color32 {
    match kind {
        TipKind::Item => item_quality_color(quality),
        TipKind::Quest => LINK_GOLD,
    }
}

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

/// The game's default chat color for a log line (ChatTypeInfo), `None` for player chat
/// and other addons' prints, which keep the normal text color.
pub fn log_text_color(channel: &str) -> Option<Color32> {
    let key = channel.strip_prefix('*')?.to_lowercase();
    Some(match key.as_str() {
        "system" | "money" | "achievement" => hex(0xffff00),
        "emote" => hex(0xff8040),
        "loot" | "currency" => hex(0x00aa00),
        "xp" => hex(0x6f6fff),
        "reputation" => hex(0x8080ff),
        "skill" => hex(0x5555ff),
        "npc" => hex(0xffff9f),
        "bossemote" => hex(0xffdd00),
        _ => return None,
    })
}

/// Color of a highlighted word in a log line ("online" green, "offline" red).
pub fn log_highlight_color(h: crate::core::channels::Highlight) -> Color32 {
    match h {
        crate::core::channels::Highlight::Good => hex(0x40ff40),
        crate::core::channels::Highlight::Bad => hex(0xff4040),
    }
}
