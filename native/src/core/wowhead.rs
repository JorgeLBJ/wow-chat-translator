//! Item data for the hover panel, "Wowhead (internet)" source: the item's tooltip from
//! Wowhead's tooltip endpoint (the one its own website widget uses):
//!
//! ```text
//! GET https://nether.wowhead.com/tooltip/item/<id>?dataEnv=<env>&locale=<locale>
//! -> {"name": "...", "quality": 5, "icon": "inv_sword_39", "tooltip": "<table>...</table>", ...}
//! ```
//!
//! Quests use the same endpoint with `tooltip/quest/<id>`; the answer then carries only
//! `name` and `tooltip` (no quality).
//!
//! Only the item or quest id (and the game version / language) is sent. Results, failures
//! included, are kept in a [`WowheadCache`] so an item is asked for once, and a failing
//! one is retried only after [`RETRY_AFTER`].

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::core::item_tips::{ItemTooltip, TipKey, TipLine, TOOLTIP_CACHE_CAP};
use crate::core::prefs::ItemTooltips;
use crate::translators::http::HttpClient;

/// Spanish (`es.wowhead.com`, esES) in Wowhead's locale numbering.
pub const LOCALE_ES: u8 = 6;

/// Request timeout.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// A failed item is asked for again only after this long.
pub const RETRY_AFTER: Duration = Duration::from_secs(180);

/// Requests running at the same time, all items together.
pub const MAX_IN_FLIGHT: usize = 4;

/// Wowhead `dataEnv` values (from `WH.dataEnv` in wow.zamimg.com/js/tooltips.js).
pub mod env {
    pub const RETAIL: u8 = 1;
    pub const PTR: u8 = 2;
    pub const BETA: u8 = 3;
    pub const CLASSIC_ERA: u8 = 4;
    pub const TBC: u8 = 5;
    pub const WRATH: u8 = 8;
    pub const CATA: u8 = 11;
    pub const MISTS: u8 = 15;
    /// "Classic Plus", Wowhead's section for WoW Forever (`wowhead.com/forever`).
    pub const FOREVER: u8 = 16;
}

/// The Wowhead data environment of a game flavor folder (`_retail_`, `_classic_`...)
/// and its shown name (from `.build.info`, e.g. "Mists of Pandaria Classic"). The
/// `_classic_` folder holds whichever progression expansion is live, so its name
/// decides; unknown folders (or none) use retail.
pub fn data_env(folder: Option<&str>, name: &str) -> u8 {
    let Some(folder) = folder else { return env::RETAIL };
    match folder.to_ascii_lowercase().as_str() {
        "_retail_" => env::RETAIL,
        "_ptr_" | "_xptr_" => env::PTR,
        "_beta_" => env::BETA,
        "_forever_" => env::FOREVER,
        "_classic_era_" | "_classic_era_ptr_" => env::CLASSIC_ERA,
        // Anniversary realms moved on to The Burning Crusade.
        "_anniversary_" => {
            if name.contains("Classic Era") {
                env::CLASSIC_ERA
            } else {
                env::TBC
            }
        }
        f if f.starts_with("_classic") => {
            if name.contains("Burning Crusade") {
                env::TBC
            } else if name.contains("Wrath") {
                env::WRATH
            } else if name.contains("Cataclysm") {
                env::CATA
            } else {
                env::MISTS
            }
        }
        _ => env::RETAIL,
    }
}

/// The tooltip endpoint for an item or a quest (a bare id is an item).
pub fn tooltip_url(key: impl Into<TipKey>, data_env: u8, locale: u8) -> String {
    let TipKey { kind, id } = key.into();
    format!("https://nether.wowhead.com/tooltip/{}/{id}?dataEnv={data_env}&locale={locale}", kind.as_str())
}

/// Sent as `User-Agent`: the app and its version, nothing about the user.
pub fn user_agent() -> String {
    format!("WoWChatTranslator/{}", env!("CARGO_PKG_VERSION"))
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    name: String,
    #[serde(default)]
    quality: Option<u8>,
    #[serde(default)]
    tooltip: String,
}

/// Reads the endpoint's JSON into a tooltip (name first, in its quality).
pub fn parse_answer(body: &str) -> Result<ItemTooltip, String> {
    let a: Answer = serde_json::from_str(body).map_err(|e| format!("bad JSON: {e}"))?;
    let name = a.name.trim();
    if name.is_empty() {
        return Err("no item name".into());
    }
    let mut lines = html_to_lines(&a.tooltip);
    if lines.first().is_none_or(|l| l.left != name) {
        lines.insert(0, TipLine { left: name.to_string(), right: None });
    }
    Ok(ItemTooltip { quality: a.quality.filter(|q| *q <= 8), lines })
}

/// Asks Wowhead for one item or quest (blocking: run it off the UI thread).
pub fn fetch(client: &dyn HttpClient, key: impl Into<TipKey>, data_env: u8) -> Result<ItemTooltip, String> {
    let headers = [("User-Agent".to_string(), user_agent()), ("Accept".to_string(), "application/json".to_string())];
    let res = client.get(&tooltip_url(key, data_env, LOCALE_ES), &headers, TIMEOUT)?;
    if !res.ok() {
        return Err(format!("HTTP {}", res.status));
    }
    parse_answer(&res.body)
}

/// Turns Wowhead tooltip HTML into lines: `<br>`, table rows and block tags end a line,
/// a `<th>` cell is the right column (e.g. "Sword" beside "One-Hand"), tags and
/// comments are dropped, entities decoded and whitespace collapsed.
pub fn html_to_lines(html: &str) -> Vec<TipLine> {
    let mut lines = Vec::new();
    let mut left = String::new();
    let mut right = String::new();
    let mut in_right = false;
    let mut skip_depth = 0usize;

    fn flush(lines: &mut Vec<TipLine>, left: &mut String, right: &mut String) {
        let l = collapse(left);
        let r = collapse(right);
        if !l.is_empty() || !r.is_empty() {
            lines.push(TipLine { left: l, right: (!r.is_empty()).then_some(r) });
        }
        left.clear();
        right.clear();
    }

    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else { break };
            let tag = &rest[1..end];
            rest = &rest[end + 1..];
            let closing = tag.starts_with('/');
            let tag_name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            match tag_name.as_str() {
                "script" | "style" => {
                    if closing {
                        skip_depth = skip_depth.saturating_sub(1);
                    } else if !tag.ends_with('/') {
                        skip_depth += 1;
                    }
                }
                "br" | "table" | "tr" | "div" | "p" | "li" | "ul" => {
                    flush(&mut lines, &mut left, &mut right);
                    in_right = false;
                }
                "th" if !closing => in_right = true,
                "td" if !closing => in_right = false,
                _ => {}
            }
            continue;
        }
        let next = rest.find('<').unwrap_or(rest.len());
        if skip_depth == 0 {
            let text = decode_entities(&rest[..next]);
            if in_right {
                right.push_str(&text);
            } else {
                left.push_str(&text);
            }
        }
        rest = &rest[next..];
    }
    flush(&mut lines, &mut left, &mut right);
    lines
}

/// Trims and turns every whitespace run (newlines, indentation, nbsp) into one space.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decodes the HTML entities Wowhead uses (named basics and numeric ones).
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..].find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let name = &rest[1..semi + 1];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                n => {
                    let num = n.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => num.parse().ok(),
                    }?;
                    char::from_u32(code)
                }
            }?;
            Some((ch, semi + 2))
        });
        match decoded {
            Some((ch, len)) => {
                out.push(ch);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// What is known about an item on Wowhead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WowheadState {
    Loading,
    Ready(ItemTooltip),
    Failed { at: Instant, error: String },
}

/// Wowhead results by [`TipKey`] (failures too), at most [`TOOLTIP_CACHE_CAP`] entries.
/// A bare `u64` key is an item.
#[derive(Debug, Default)]
pub struct WowheadCache {
    map: HashMap<TipKey, WowheadState>,
    order: VecDeque<TipKey>,
}

impl WowheadCache {
    pub fn get(&self, key: impl Into<TipKey>) -> Option<&WowheadState> {
        self.map.get(&key.into())
    }

    fn in_flight(&self) -> usize {
        self.map.values().filter(|s| matches!(s, WowheadState::Loading)).count()
    }

    /// True when a request for `id` should start now, and marks it loading: never while
    /// one is running or after a success, after a failure only once [`RETRY_AFTER`] has
    /// passed, and never beyond [`MAX_IN_FLIGHT`] requests at once.
    pub fn begin(&mut self, key: impl Into<TipKey>, now: Instant) -> bool {
        let id = key.into();
        match self.map.get(&id) {
            Some(WowheadState::Loading | WowheadState::Ready(_)) => return false,
            Some(WowheadState::Failed { at, .. }) if now.saturating_duration_since(*at) < RETRY_AFTER => return false,
            _ => {}
        }
        if self.in_flight() >= MAX_IN_FLIGHT {
            return false;
        }
        self.put(id, WowheadState::Loading);
        true
    }

    /// Stores a request's outcome.
    pub fn finish(&mut self, key: impl Into<TipKey>, result: Result<ItemTooltip, String>, now: Instant) {
        let state = match result {
            Ok(tip) => WowheadState::Ready(tip),
            Err(error) => WowheadState::Failed { at: now, error },
        };
        self.put(key.into(), state);
    }

    /// Stores a known tooltip (sample data for screenshots).
    pub fn insert_ready(&mut self, key: impl Into<TipKey>, tip: ItemTooltip) {
        self.put(key.into(), WowheadState::Ready(tip));
    }

    fn put(&mut self, id: TipKey, state: WowheadState) {
        if self.map.insert(id, state).is_some() {
            self.order.retain(|&k| k != id);
        }
        self.order.push_back(id);
        while self.order.len() > TOOLTIP_CACHE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// The source note of Wowhead data in the panel.
pub const WOWHEAD_NOTE: &str = "Wowhead";
/// The source note of addon data shown because Wowhead failed.
pub const ADDON_NOTE: &str = "WoW (addon)";
/// Shown while waiting for data.
pub const LOADING_TEXT: &str = "Cargando datos…";
/// Shown when Wowhead failed and the addon sent nothing.
pub const NO_WOWHEAD_TEXT: &str = "Sin datos de Wowhead";

/// What the item panel shows for one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelView<'a> {
    /// Tooltip lines, with a small source note when set.
    Tip { tip: &'a ItemTooltip, note: Option<&'static str> },
    /// Only the name and a message ("Cargando datos…", "Sin datos de Wowhead").
    Message(&'static str),
}

impl<'a> PanelView<'a> {
    pub fn tip(&self) -> Option<&'a ItemTooltip> {
        match self {
            PanelView::Tip { tip, .. } => Some(tip),
            PanelView::Message(_) => None,
        }
    }
}

/// The panel for an item given the selected source: `Addon` shows the addon's data or
/// "Cargando datos…" (as before Wowhead existed); `Wowhead` shows Wowhead's data, or
/// while it loads "Cargando datos…", and after a failure the addon's data when there is
/// some, else "Sin datos de Wowhead". `None` when the panel is off.
pub fn panel_view<'a>(
    mode: ItemTooltips,
    addon: Option<&'a ItemTooltip>,
    wowhead: Option<&'a WowheadState>,
) -> Option<PanelView<'a>> {
    match mode {
        ItemTooltips::Off => None,
        ItemTooltips::Addon => Some(match addon {
            Some(tip) => PanelView::Tip { tip, note: None },
            None => PanelView::Message(LOADING_TEXT),
        }),
        ItemTooltips::Wowhead => Some(match (wowhead, addon) {
            (Some(WowheadState::Ready(tip)), _) => PanelView::Tip { tip, note: Some(WOWHEAD_NOTE) },
            (Some(WowheadState::Failed { .. }), Some(tip)) => PanelView::Tip { tip, note: Some(ADDON_NOTE) },
            (Some(WowheadState::Failed { .. }), None) => PanelView::Message(NO_WOWHEAD_TEXT),
            (Some(WowheadState::Loading) | None, _) => PanelView::Message(LOADING_TEXT),
        }),
    }
}
