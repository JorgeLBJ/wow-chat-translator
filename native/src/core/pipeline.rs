//! Turns new chat messages into overlay items and translation jobs, unless translation
//! is paused. Paused (and failed) messages are kept so the user can translate any one of
//! them on demand by clicking it. Port of `app/src/core/pipeline.js` for exact
//! pixel-strip text (`ocrFixes = false`).
//!
//! The pipeline does no I/O: it queues [`UiEvent`]s and [`Job`]s that the caller drains
//! with [`Pipeline::take_events`] / [`Pipeline::take_jobs`]; whoever runs a job (a worker
//! thread, usually) hands the result back through [`Pipeline::complete`].

use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::channels::{channel_tab, is_log_channel};
use crate::core::emoji::replace_shortcodes;
use crate::core::language::looks_spanish;
use crate::core::links::{marked_link_refs, only_links, protect_marked_links, restore_links, strip_link_markers, LinkRef};
use crate::strip::Message;

/// Pending entries kept for on-demand translation; the oldest are dropped first.
pub const KEEP: usize = 300;

/// A translator's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Translation {
    pub text: String,
    /// Detected source language, or "auto" when unknown.
    pub source: String,
    pub provider: Option<String>,
    pub fallback_reason: Option<String>,
}

/// Anything that can translate text into a target language ("es", "en").
pub trait Translator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Done,
    Paused,
    Error,
}

/// One chat line shown in the overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayItem {
    pub id: u64,
    /// Milliseconds since the Unix epoch.
    pub time: u64,
    pub channel: String,
    pub tab: Option<&'static str>,
    pub class: Option<String>,
    pub player: String,
    pub original: String,
    pub translated: Option<String>,
    pub lang: Option<String>,
    pub status: Status,
    pub provider: Option<String>,
    pub fallback_reason: Option<String>,
    pub error: Option<String>,
    /// Type and id of the links that carry one (addon 0.3.0+), in order.
    pub links: Vec<LinkRef>,
}

/// A change to an existing item. Which fields matter depends on `status`:
/// `Pending` only changes the status, `Error` sets `error`, `Done` sets the
/// translation fields (see [`OverlayItem::apply`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemPatch {
    pub status: Status,
    pub translated: Option<String>,
    pub lang: Option<String>,
    pub provider: Option<String>,
    pub fallback_reason: Option<String>,
    pub error: Option<String>,
}

impl ItemPatch {
    pub fn status(status: Status) -> Self {
        Self { status, translated: None, lang: None, provider: None, fallback_reason: None, error: None }
    }

    pub fn error(message: &str) -> Self {
        Self { error: Some(message.to_string()), ..Self::status(Status::Error) }
    }
}

impl OverlayItem {
    /// Applies an update produced by the pipeline.
    pub fn apply(&mut self, patch: &ItemPatch) {
        self.status = patch.status;
        match patch.status {
            Status::Done => {
                self.translated = patch.translated.clone();
                self.lang = patch.lang.clone();
                self.provider = patch.provider.clone();
                self.fallback_reason = patch.fallback_reason.clone();
                self.error = None;
            }
            Status::Error => self.error = patch.error.clone(),
            Status::Pending | Status::Paused => self.error = None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    Add(OverlayItem),
    Update(u64, ItemPatch),
}

/// A translation to run: the masked text (links replaced by placeholders).
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub id: u64,
    pub text: String,
    pub target: String,
}

#[derive(Clone, Debug)]
struct Entry {
    original: String,
    masked: String,
    links: Vec<String>,
}

pub struct Pipeline {
    target: String,
    keep: usize,
    paused: bool,
    next_id: u64,
    now: Box<dyn Fn() -> u64 + Send>,
    /// Not translated yet. Ids grow, so key order is insertion order.
    pending: BTreeMap<u64, Entry>,
    /// Requested and not completed; keeps its entry in case it is evicted meanwhile.
    in_flight: HashMap<u64, Entry>,
    events: Vec<UiEvent>,
    jobs: Vec<Job>,
}

/// Drops the non-player log lines (loot, system, NPC...) unless the log is on, so with
/// the option off they are neither stored nor shown.
pub fn filter_log_messages(messages: &[Message], show_log: bool) -> Vec<Message> {
    messages.iter().filter(|m| show_log || !is_log_channel(&m.channel)).cloned().collect()
}

fn epoch_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Pipeline {
    pub fn new(target: &str) -> Self {
        Self {
            target: target.to_string(),
            keep: KEEP,
            paused: false,
            next_id: 1,
            now: Box::new(epoch_millis),
            pending: BTreeMap::new(),
            in_flight: HashMap::new(),
            events: Vec::new(),
            jobs: Vec::new(),
        }
    }

    /// Replaces the clock used for item times (milliseconds).
    pub fn set_clock(&mut self, now: Box<dyn Fn() -> u64 + Send>) {
        self.now = now;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn add_messages(&mut self, messages: &[Message]) {
        for msg in messages {
            let text = replace_shortcodes(&msg.text); // :sob: -> emoji
            // Links are never translated: only the ones the addon marked are masked.
            let original = strip_link_markers(&text);
            let (masked, links) = protect_marked_links(&text);
            let mut item = OverlayItem {
                id: self.next_id,
                time: (self.now)(),
                channel: msg.channel.clone(),
                tab: channel_tab(&msg.channel),
                class: msg.class.clone(),
                player: msg.player.clone(),
                original: original.clone(),
                translated: None,
                lang: None,
                status: Status::Pending,
                provider: None,
                fallback_reason: None,
                error: None,
                links: marked_link_refs(&text),
            };
            self.next_id += 1;
            // Log lines (loot, system, NPC...) already come in the client's language.
            if is_log_channel(&msg.channel) {
                item.status = Status::Done;
                self.events.push(UiEvent::Add(item));
                continue;
            }
            if only_links(&masked) || looks_spanish(&masked) {
                item.lang = Some("es".into());
                item.status = Status::Done;
                self.events.push(UiEvent::Add(item));
                continue;
            }
            let id = item.id;
            self.remember(id, Entry { original, masked, links });
            if self.paused {
                item.status = Status::Paused;
                self.events.push(UiEvent::Add(item));
                continue;
            }
            self.events.push(UiEvent::Add(item));
            self.request(id);
        }
    }

    /// Translates one paused or failed message. False when there is nothing to do
    /// (unknown id, already translated, or already being translated).
    pub fn translate_by_id(&mut self, id: u64) -> bool {
        if !self.pending.contains_key(&id) || self.in_flight.contains_key(&id) {
            return false;
        }
        self.events.push(UiEvent::Update(id, ItemPatch::status(Status::Pending)));
        self.request(id);
        true
    }

    /// Hands back the result of a job. False when the id was not in flight.
    pub fn complete(&mut self, id: u64, result: Result<Translation, String>) -> bool {
        let Some(entry) = self.in_flight.remove(&id) else { return false };
        let patch = match result {
            Ok(res) => {
                self.pending.remove(&id);
                let text = restore_links(&res.text, &entry.links);
                let same = res.source == self.target || text.trim().to_lowercase() == entry.original.trim().to_lowercase();
                ItemPatch {
                    status: Status::Done,
                    translated: if same { None } else { Some(text) },
                    lang: if res.source == "auto" { None } else { Some(res.source) },
                    provider: res.provider,
                    fallback_reason: res.fallback_reason,
                    error: None,
                }
            }
            Err(message) => ItemPatch::error(&message),
        };
        self.events.push(UiEvent::Update(id, patch));
        true
    }

    /// Items to add and updates to apply, oldest first.
    pub fn take_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.events)
    }

    /// Translations to run; they stay in flight until [`Pipeline::complete`].
    pub fn take_jobs(&mut self) -> Vec<Job> {
        std::mem::take(&mut self.jobs)
    }

    fn remember(&mut self, id: u64, entry: Entry) {
        self.pending.insert(id, entry);
        if self.pending.len() > self.keep {
            self.pending.pop_first();
        }
    }

    fn request(&mut self, id: u64) {
        let entry = self.pending[&id].clone();
        self.jobs.push(Job { id, text: entry.masked.clone(), target: self.target.clone() });
        self.in_flight.insert(id, entry);
    }
}
