//! Item data for the hover panel, "WoW (addon)" source. Since addon 0.3.1 the addon also
//! sends, at low priority and once per item per game session, the game's own tooltip of
//! every item linked in chat, as a frame on the `*Item` channel:
//!
//! ```text
//! v1 SEP *Item SEP (empty class) SEP <item id> SEP [q<quality>\n]<name>\n<line>[\t<right>]\n...
//! ```
//!
//! `quality` is the item quality digit (0 poor ... 5 legendary, up to 8); the first line
//! after it is the item name. A right column (e.g. "Axe" beside "Two-Hand") follows its
//! line after a tab.
//!
//! Since addon 0.3.3 quest links get the same frames on the `*Quest` channel, with the
//! quest id as player field and the quest tooltip (title first, no quality line):
//!
//! ```text
//! v1 SEP *Quest SEP (empty class) SEP <quest id> SEP <title>\n<line>[\t<right>]\n...
//! ```
//!
//! Those frames are never chat rows: [`split_tip_frames`] takes them out before the
//! pipeline and they fill an [`ItemTooltipCache`], keyed by [`TipKey`] (kind and id) so an
//! item and a quest with the same id never collide.

use std::collections::{HashMap, VecDeque};

use crate::core::links::LinkRef;
use crate::strip::Message;

/// The channel label of tooltip frames ("*" cannot appear in a real channel name).
pub const ITEM_CHANNEL: &str = "*Item";

/// The channel label of quest tooltip frames.
pub const QUEST_CHANNEL: &str = "*Quest";

/// The link kinds that have a hover panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TipKind {
    Item,
    Quest,
}

impl TipKind {
    /// The link kind (`item:<id>`, `quest:<id>`), also Wowhead's tooltip path.
    pub fn as_str(self) -> &'static str {
        match self {
            TipKind::Item => "item",
            TipKind::Quest => "quest",
        }
    }

    /// The kind of a link's ref text; other kinds (spell, achievement...) have none.
    pub fn of_link_kind(kind: &str) -> Option<Self> {
        match kind {
            "item" => Some(TipKind::Item),
            "quest" => Some(TipKind::Quest),
            _ => None,
        }
    }
}

/// What a tooltip is about: an item or a quest, and its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TipKey {
    pub kind: TipKind,
    pub id: u64,
}

impl TipKey {
    pub fn item(id: u64) -> Self {
        Self { kind: TipKind::Item, id }
    }

    pub fn quest(id: u64) -> Self {
        Self { kind: TipKind::Quest, id }
    }

    /// The key of a link with a hover panel (item or quest), else `None`.
    pub fn of_link(link: &LinkRef) -> Option<Self> {
        TipKind::of_link_kind(&link.kind).map(|kind| Self { kind, id: link.id })
    }
}

/// A bare id is an item, the only kind before quests had tooltips.
impl From<u64> for TipKey {
    fn from(id: u64) -> Self {
        Self::item(id)
    }
}

/// Items kept in the cache; the oldest are dropped first.
pub const TOOLTIP_CACHE_CAP: usize = 500;

/// One tooltip line: the left text and, for two-column lines, the right one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TipLine {
    pub left: String,
    pub right: Option<String>,
}

/// An item's tooltip as the game shows it; the first line is the item name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemTooltip {
    /// Item quality (0 poor, 1 common, 2 uncommon, 3 rare, 4 epic, 5 legendary...).
    pub quality: Option<u8>,
    pub lines: Vec<TipLine>,
}

impl ItemTooltip {
    pub fn name(&self) -> Option<&str> {
        self.lines.first().map(|l| l.left.as_str())
    }
}

/// True for the addon's tooltip frames (exactly `*Item`; a player channel "Item" stays chat).
pub fn is_item_channel(channel: &str) -> bool {
    channel == ITEM_CHANNEL
}

/// True for the addon's quest tooltip frames (exactly `*Quest`).
pub fn is_quest_channel(channel: &str) -> bool {
    channel == QUEST_CHANNEL
}

/// Parses a tooltip frame's player field (the item id) and text; `None` when the id is
/// not a number or there is no item name.
pub fn parse_item_tooltip(id: &str, text: &str) -> Option<(u64, ItemTooltip)> {
    parse_tooltip(id, text, true)
}

/// Parses a quest tooltip frame (quest id, then title and lines; no quality line).
pub fn parse_quest_tooltip(id: &str, text: &str) -> Option<(u64, ItemTooltip)> {
    parse_tooltip(id, text, false)
}

fn parse_tooltip(id: &str, text: &str, with_quality: bool) -> Option<(u64, ItemTooltip)> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id: u64 = id.parse().ok()?;
    let mut rows = text.split('\n').peekable();
    let quality = rows.peek().filter(|_| with_quality).and_then(|first| {
        let digit = first.trim().strip_prefix('q')?;
        match digit.as_bytes() {
            [d @ b'0'..=b'8'] => Some(d - b'0'),
            _ => None,
        }
    });
    if quality.is_some() {
        rows.next();
    }
    let lines: Vec<TipLine> = rows
        .filter_map(|row| {
            let (left, right) = match row.split_once('\t') {
                Some((l, r)) => (l.trim(), Some(r.trim()).filter(|r| !r.is_empty())),
                None => (row.trim(), None),
            };
            (!left.is_empty() || right.is_some()).then(|| TipLine { left: left.to_string(), right: right.map(str::to_string) })
        })
        .collect();
    if lines.first().is_none_or(|l| l.left.is_empty()) {
        return None;
    }
    Some((id, ItemTooltip { quality, lines }))
}

/// Takes the tooltip frames (`*Item`, `*Quest`) out of a batch of decoded messages:
/// returns the other messages (in order) and the valid tooltips by key. Garbled tooltip
/// frames are dropped too, so a tooltip frame never becomes a chat row.
pub fn split_tip_frames(messages: &[Message]) -> (Vec<Message>, Vec<(TipKey, ItemTooltip)>) {
    let mut rest = Vec::with_capacity(messages.len());
    let mut tips = Vec::new();
    for m in messages {
        if is_item_channel(&m.channel) {
            tips.extend(parse_item_tooltip(&m.player, &m.text).map(|(id, t)| (TipKey::item(id), t)));
        } else if is_quest_channel(&m.channel) {
            tips.extend(parse_quest_tooltip(&m.player, &m.text).map(|(id, t)| (TipKey::quest(id), t)));
        } else {
            rest.push(m.clone());
        }
    }
    (rest, tips)
}

/// Item and quest tooltips by [`TipKey`], at most [`TOOLTIP_CACHE_CAP`]; inserting a key
/// again replaces it and makes it the newest. A bare `u64` key is an item.
#[derive(Debug, Default)]
pub struct ItemTooltipCache {
    map: HashMap<TipKey, ItemTooltip>,
    order: VecDeque<TipKey>,
}

impl ItemTooltipCache {
    pub fn insert(&mut self, key: impl Into<TipKey>, tip: ItemTooltip) {
        let key = key.into();
        if self.map.insert(key, tip).is_some() {
            self.order.retain(|&k| k != key);
        }
        self.order.push_back(key);
        while self.order.len() > TOOLTIP_CACHE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    pub fn get(&self, key: impl Into<TipKey>) -> Option<&ItemTooltip> {
        self.map.get(&key.into())
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// A piece of a message's text: plain text, or a link's shown name with its ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Link(String, LinkRef),
}

/// Splits `text` into plain and link segments, finding each link by its shown name
/// (translations keep link names as they were). At the same place the longest name wins;
/// text matching no link (e.g. brackets a player typed) stays plain.
pub fn link_segments(text: &str, links: &[LinkRef]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let hit = links.iter().filter(|l| !l.name.is_empty() && rest.starts_with(&l.name)).max_by_key(|l| l.name.len());
        if let Some(link) = hit {
            if plain_start < i {
                out.push(Segment::Text(text[plain_start..i].to_string()));
            }
            out.push(Segment::Link(link.name.clone(), link.clone()));
            i += link.name.len();
            plain_start = i;
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    if plain_start < text.len() {
        out.push(Segment::Text(text[plain_start..].to_string()));
    }
    out
}

/// The text split at its item and quest links (the links with a hover panel), or `None`
/// when it has none to draw apart (the row is then drawn exactly as without tooltips).
/// Other link kinds (spell, achievement...) stay plain text.
pub fn tip_segments(text: &str, links: &[LinkRef]) -> Option<Vec<Segment>> {
    let tipped: Vec<LinkRef> = links.iter().filter(|l| TipKey::of_link(l).is_some()).cloned().collect();
    if tipped.is_empty() {
        return None;
    }
    let segs = link_segments(text, &tipped);
    segs.iter().any(|s| matches!(s, Segment::Link(..))).then_some(segs)
}
