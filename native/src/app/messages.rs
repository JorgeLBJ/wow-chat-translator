//! The overlay's message list: the last [`MAX_ITEMS`] items, the active tab and the
//! unread counters, plus the per-item display rules (badge, clickable, which text).
//! Port of the state handling in `app/src/renderer/overlay.js`; no drawing here.

use std::collections::VecDeque;

use crate::core::channels::LOG_TAB;
use crate::core::pipeline::{OverlayItem, Status, UiEvent};

pub const MAX_ITEMS: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    All,
    General,
    Trade,
    Guild,
    Group,
    Whisper,
    /// Non-player chat (loot, system, NPC...); shown only when the log option is on.
    Log,
}

const TABS: usize = 7;

impl Tab {
    pub const ALL: [Tab; TABS] = [Tab::All, Tab::General, Tab::Trade, Tab::Guild, Tab::Group, Tab::Whisper, Tab::Log];

    /// The pipeline's tab key (`OverlayItem::tab`); `None` for "Todos".
    pub fn key(self) -> Option<&'static str> {
        match self {
            Tab::All => None,
            Tab::General => Some("general"),
            Tab::Trade => Some("trade"),
            Tab::Guild => Some("guild"),
            Tab::Group => Some("group"),
            Tab::Whisper => Some("whisper"),
            Tab::Log => Some(LOG_TAB),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "Todos",
            Tab::General => "General",
            Tab::Trade => "Comercio",
            Tab::Guild => "Hermandad",
            Tab::Group => "Grupo",
            Tab::Whisper => "Susurros",
            Tab::Log => "Registro",
        }
    }

    pub fn from_key(key: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.key() == Some(key))
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

/// Translated or original text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Translated,
    Original,
}

/// One item plus its precomputed local time ("HH:MM").
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub item: OverlayItem,
    pub time_label: String,
    /// How many identical consecutive log lines this row stands for (shown as ×N).
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct MessageList {
    rows: VecDeque<Row>,
    unread: [u32; TABS],
    active: Tab,
}

impl Default for MessageList {
    fn default() -> Self {
        Self { rows: VecDeque::new(), unread: [0; TABS], active: Tab::All }
    }
}

impl MessageList {
    /// Applies one pipeline event. `time_label` formats an item's time (ms).
    pub fn apply(&mut self, event: UiEvent, time_label: &dyn Fn(u64) -> String) {
        match event {
            UiEvent::Add(item) => self.add(item, time_label),
            UiEvent::Update(id, patch) => {
                if let Some(row) = self.rows.iter_mut().rev().find(|r| r.item.id == id) {
                    row.item.apply(&patch);
                }
            }
        }
    }

    fn add(&mut self, item: OverlayItem, time_label: &dyn Fn(u64) -> String) {
        // Same rule as overlay.js: count only while a specific tab is open; on "Todos"
        // everything is in view, so the tab's count resets.
        if let Some(tab) = item.tab.and_then(Tab::from_key) {
            if self.active != Tab::All && tab != self.active {
                self.unread[tab.index()] += 1;
            } else if self.active == Tab::All {
                self.unread[tab.index()] = 0;
            }
        }
        let time_label = time_label(item.time);
        // Identical consecutive log lines (an addon repeating itself, the same loot) stack
        // into the previous row: ×N and the latest time. Player chat never stacks.
        if item.tab == Some(LOG_TAB) {
            if let Some(last) = self.rows.back_mut() {
                let same = last.item.tab == Some(LOG_TAB)
                    && last.item.channel == item.channel
                    && last.item.player == item.player
                    && last.item.original == item.original;
                if same {
                    last.count += 1;
                    last.time_label = time_label;
                    return;
                }
            }
        }
        self.rows.push_back(Row { item, time_label, count: 1 });
        while self.rows.len() > MAX_ITEMS {
            self.rows.pop_front();
        }
    }

    pub fn active(&self) -> Tab {
        self.active
    }

    pub fn set_active(&mut self, tab: Tab) {
        self.active = tab;
        self.unread[tab.index()] = 0;
    }

    /// The count to show next to a tab, if any.
    pub fn unread_badge(&self, tab: Tab) -> Option<u32> {
        let n = self.unread[tab.index()];
        (n > 0 && tab != self.active && tab != Tab::All).then_some(n)
    }

    pub fn is_visible(&self, item: &OverlayItem) -> bool {
        self.active == Tab::All || item.tab == self.active.key()
    }

    /// Rows of the active tab, oldest first.
    pub fn visible(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| self.is_visible(&r.item))
    }

    /// Marks one item as being translated (immediate feedback on click).
    pub fn mark_pending(&mut self, id: u64) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.item.id == id) {
            row.item.status = Status::Pending;
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// Removes the log rows and leaves the "Registro" tab (the log option was turned off).
    pub fn drop_log(&mut self) {
        self.rows.retain(|r| r.item.tab != Some(LOG_TAB));
        self.unread[Tab::Log.index()] = 0;
        if self.active == Tab::Log {
            self.active = Tab::All;
        }
    }
}

/// True when the translated text is the one to show.
pub fn shows_translation(item: &OverlayItem, mode: Mode) -> bool {
    mode == Mode::Translated && item.translated.is_some()
}

/// The mode one row is drawn in: the global mode, or the other one when the user
/// flipped that row from its context menu.
pub fn row_mode(global: Mode, flipped: bool) -> Mode {
    match (global, flipped) {
        (m, false) => m,
        (Mode::Translated, true) => Mode::Original,
        (Mode::Original, true) => Mode::Translated,
    }
}

/// The context-menu entry that flips a row drawn in `mode`; `None` when the message
/// has no translation to switch to.
pub fn flip_label(item: &OverlayItem, mode: Mode) -> Option<&'static str> {
    item.translated.as_ref()?;
    Some(if shows_translation(item, mode) { "Ver original" } else { "Ver traducción" })
}

/// Paused and failed messages can be translated with a click.
pub fn can_translate(item: &OverlayItem) -> bool {
    matches!(item.status, Status::Paused | Status::Error)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Lang,
    Paused,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub text: String,
    pub kind: BadgeKind,
    /// Hover text (provider, error...).
    pub tip: Option<String>,
}

/// The small badge after the player name (language, paused, error).
pub fn badge(item: &OverlayItem, show_translated: bool) -> Option<Badge> {
    match item.status {
        Status::Error => Some(Badge {
            text: "error · traducir".into(),
            kind: BadgeKind::Error,
            tip: Some(format!("{} (clic para reintentar)", item.error.as_deref().unwrap_or(""))),
        }),
        Status::Paused => Some(Badge { text: "⏸ traducir".into(), kind: BadgeKind::Paused, tip: None }),
        _ => {
            let tip = item.provider.as_ref().map(|p| match &item.fallback_reason {
                Some(reason) => format!("{p} (LLM falló: {reason})"),
                None => p.clone(),
            });
            match &item.lang {
                Some(lang) => {
                    let up = lang.to_uppercase();
                    let text = if show_translated { format!("{up}→ES") } else { up };
                    Some(Badge { text, kind: BadgeKind::Lang, tip })
                }
                None if show_translated => Some(Badge { text: "→ES".into(), kind: BadgeKind::Lang, tip }),
                None => None,
            }
        }
    }
}

