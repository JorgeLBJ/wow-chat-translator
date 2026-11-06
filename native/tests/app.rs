//! Pure logic behind the overlay window: presence gate, game watcher, message list
//! (unread counters, item cap, badges) and formatting helpers.

use wow_chat_translator::app::debug_log::{iso_utc, DebugLog};
use wow_chat_translator::app::game_watcher::{GameChange, GameWatcher};
use wow_chat_translator::app::messages::{badge, can_translate, shows_translation, BadgeKind, MessageList, Mode, Tab, MAX_ITEMS};
use wow_chat_translator::app::presence::PresenceGate;
use wow_chat_translator::app::style::{channel_label, class_color, hhmm, tab_color, translator_label, ReaderState};
use wow_chat_translator::core::pipeline::{ItemPatch, OverlayItem, Status, UiEvent};
use wow_chat_translator::core::prefs::TranslatorSettings;

// Presence gate (port of app/test/presence-gate.test.js) ---------------------------

/// Fake clock: `advance` polls the gate every 5 ms like the reader loop would.
struct Sim {
    gate: PresenceGate,
    now: u64,
    focused: bool,
    changes: Vec<bool>,
}

impl Sim {
    fn new(focused: bool) -> Self {
        Self { gate: PresenceGate::new(2000), now: 0, focused, changes: Vec::new() }
    }
    fn seen(&mut self) {
        self.changes.extend(self.gate.seen(self.now));
    }
    fn lost(&mut self) {
        self.gate.lost(self.now);
    }
    fn advance(&mut self, ms: u64) {
        let end = self.now + ms;
        while self.now < end {
            self.now = (self.now + 5).min(end);
            self.changes.extend(self.gate.poll(self.now, self.focused));
        }
    }
}

#[test]
fn gate_shows_right_away_when_the_strip_appears() {
    let mut s = Sim::new(false);
    s.seen();
    assert_eq!(s.changes, vec![true]);
}

#[test]
fn gate_does_not_hide_on_a_short_absence() {
    let mut s = Sim::new(false);
    s.seen();
    s.lost();
    s.advance(1500);
    s.seen();
    s.advance(5000);
    assert_eq!(s.changes, vec![true]);
}

#[test]
fn gate_hides_after_the_strip_has_been_gone_for_the_whole_delay() {
    let mut s = Sim::new(false);
    s.seen();
    s.lost();
    s.advance(1995);
    assert_eq!(s.changes, vec![true]);
    s.advance(5);
    assert_eq!(s.changes, vec![true, false]);
}

#[test]
fn gate_never_hides_while_the_window_has_the_focus() {
    let mut s = Sim::new(true);
    s.seen();
    s.lost();
    s.advance(10_000);
    assert_eq!(s.changes, vec![true]);
    // Focus leaves: it hides one delay later at most, not at once.
    s.focused = false;
    s.advance(2000);
    assert_eq!(s.changes, vec![true, false]);
}

#[test]
fn gate_cannot_flip_flop_faster_than_the_delay() {
    let mut s = Sim::new(false);
    s.seen();
    for _ in 0..40 {
        s.lost();
        s.advance(60);
        s.seen();
        s.advance(60);
    }
    assert_eq!(s.changes, vec![true]);
}

#[test]
fn gate_does_not_repeat_a_state_it_already_reported() {
    let mut s = Sim::new(false);
    s.seen();
    s.seen();
    assert_eq!(s.changes, vec![true]);
    s.lost();
    s.advance(2000);
    s.lost();
    s.advance(5000);
    assert_eq!(s.changes, vec![true, false]);
}

#[test]
fn gate_repeated_lost_does_not_restart_the_delay() {
    let mut s = Sim::new(false);
    s.seen();
    for _ in 0..40 {
        s.lost();
        s.advance(50);
    }
    assert_eq!(s.changes, vec![true, false]);
}

#[test]
fn gate_shows_again_at_once_after_a_hide() {
    let mut s = Sim::new(false);
    s.seen();
    s.lost();
    s.advance(2000);
    s.seen();
    assert_eq!(s.changes, vec![true, false, true]);
}

// Game watcher ------------------------------------------------------------------

#[test]
fn game_watcher_reports_only_transitions() {
    let mut w = GameWatcher::default();
    assert_eq!(w.update(false), None);
    assert_eq!(w.update(true), Some(GameChange::Started));
    assert!(w.running());
    assert_eq!(w.update(true), None);
    assert_eq!(w.update(false), Some(GameChange::Exited));
    assert_eq!(w.update(true), Some(GameChange::Started));
}

// Message list ------------------------------------------------------------------

fn item(id: u64, tab: Option<&'static str>) -> OverlayItem {
    OverlayItem {
        id,
        time: 0,
        channel: tab.unwrap_or("say").to_string(),
        tab,
        class: None,
        player: "Bob".into(),
        original: format!("hello {id}"),
        translated: None,
        lang: None,
        status: Status::Pending,
        provider: None,
        fallback_reason: None,
        error: None,
    }
}

fn add(list: &mut MessageList, id: u64, tab: Option<&'static str>) {
    list.apply(UiEvent::Add(item(id, tab)), &|_| "12:00".into());
}

#[test]
fn unread_counts_only_while_another_tab_is_open() {
    let mut list = MessageList::default();
    add(&mut list, 1, Some("trade"));
    assert_eq!(list.unread_badge(Tab::Trade), None, "the Todos tab shows everything: nothing unread");
    list.set_active(Tab::Guild);
    add(&mut list, 2, Some("trade"));
    add(&mut list, 3, Some("trade"));
    add(&mut list, 4, Some("guild"));
    add(&mut list, 5, None);
    assert_eq!(list.unread_badge(Tab::Trade), Some(2));
    assert_eq!(list.unread_badge(Tab::Guild), None, "active tab has no badge");
    assert_eq!(list.unread_badge(Tab::All), None, "Todos never has a badge");
    list.set_active(Tab::Trade);
    assert_eq!(list.unread_badge(Tab::Trade), None, "opening a tab clears its count");
}

#[test]
fn unread_resets_for_a_tab_when_a_message_arrives_on_todos() {
    let mut list = MessageList::default();
    list.set_active(Tab::Guild);
    add(&mut list, 1, Some("trade"));
    list.set_active(Tab::All);
    add(&mut list, 2, Some("trade"));
    list.set_active(Tab::Guild);
    assert_eq!(list.unread_badge(Tab::Trade), None);
}

#[test]
fn list_keeps_at_most_300_items_dropping_the_oldest() {
    let mut list = MessageList::default();
    for id in 1..=(MAX_ITEMS as u64 + 5) {
        add(&mut list, id, None);
    }
    assert_eq!(MAX_ITEMS, 300);
    assert_eq!(list.len(), 300);
    let ids: Vec<u64> = list.visible().map(|r| r.item.id).collect();
    assert_eq!(ids.first(), Some(&6));
    assert_eq!(ids.last(), Some(&305));
}

#[test]
fn list_filters_by_active_tab() {
    let mut list = MessageList::default();
    add(&mut list, 1, Some("trade"));
    add(&mut list, 2, Some("guild"));
    add(&mut list, 3, None);
    assert_eq!(list.visible().count(), 3);
    list.set_active(Tab::Guild);
    let ids: Vec<u64> = list.visible().map(|r| r.item.id).collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn list_applies_updates_and_ignores_unknown_ids() {
    let mut list = MessageList::default();
    add(&mut list, 1, None);
    let done = ItemPatch {
        status: Status::Done,
        translated: Some("hola".into()),
        lang: Some("en".into()),
        provider: Some("Google".into()),
        fallback_reason: None,
        error: None,
    };
    list.apply(UiEvent::Update(1, done), &|_| String::new());
    list.apply(UiEvent::Update(99, ItemPatch::error("x")), &|_| String::new());
    let row = list.visible().next().unwrap();
    assert_eq!(row.item.status, Status::Done);
    assert_eq!(row.item.translated.as_deref(), Some("hola"));
    assert_eq!(row.time_label, "12:00");
    assert_eq!(list.len(), 1);
    list.clear();
    assert_eq!(list.len(), 0);
}

#[test]
fn badges_follow_the_overlay_rules() {
    let mut it = item(1, None);
    it.status = Status::Error;
    assert_eq!(badge(&it, false).map(|b| (b.text, b.kind)), Some(("error · traducir".to_string(), BadgeKind::Error)));
    it.status = Status::Paused;
    assert_eq!(badge(&it, false).map(|b| (b.text, b.kind)), Some(("⏸ traducir".to_string(), BadgeKind::Paused)));
    it.status = Status::Done;
    it.lang = Some("en".into());
    assert_eq!(badge(&it, true).unwrap().text, "EN→ES");
    assert_eq!(badge(&it, false).unwrap().text, "EN");
    it.lang = None;
    assert_eq!(badge(&it, true).unwrap().text, "→ES");
    assert_eq!(badge(&it, false), None);
}

#[test]
fn translation_is_shown_only_in_translated_mode_when_present() {
    let mut it = item(1, None);
    assert!(!shows_translation(&it, Mode::Translated));
    it.translated = Some("hola".into());
    assert!(shows_translation(&it, Mode::Translated));
    assert!(!shows_translation(&it, Mode::Original));
}

#[test]
fn only_paused_or_failed_messages_are_clickable() {
    let mut it = item(1, None);
    for (status, clickable) in [(Status::Pending, false), (Status::Done, false), (Status::Paused, true), (Status::Error, true)] {
        it.status = status;
        assert_eq!(can_translate(&it), clickable, "{status:?}");
    }
}

// Formatting -------------------------------------------------------------------------

#[test]
fn hhmm_uses_the_local_offset() {
    // 2026-11-06 12:21:30 UTC
    let ms = 1_793_967_690_000;
    assert_eq!(hhmm(ms, 0), "12:21");
    assert_eq!(hhmm(ms, -300), "07:21");
    assert_eq!(hhmm(ms, 690), "23:51");
    assert_eq!(hhmm(ms, -780), "23:21");
}

#[test]
fn colors_match_the_electron_tables() {
    assert_eq!(class_color(Some("MAGE")).to_hex(), "#3fc7ebff");
    assert_eq!(class_color(Some("DEATHKNIGHT")).to_hex(), "#c41e3aff");
    assert_eq!(class_color(None).to_hex(), "#d0d4daff");
    assert_eq!(class_color(Some("UNKNOWN")).to_hex(), "#d0d4daff");
    assert_eq!(tab_color(Some("trade")).to_hex(), "#d38b8bff");
    assert_eq!(tab_color(Some("whisper")).to_hex(), "#ff80ffff");
    assert_eq!(tab_color(None).to_hex(), "#9aa3adff");
}

#[test]
fn channel_labels_are_spanish() {
    assert_eq!(channel_label("Trade"), "Comercio");
    assert_eq!(channel_label("officer"), "Oficial");
    assert_eq!(channel_label("say"), "Decir");
    assert_eq!(channel_label("Custom Channel"), "Custom Channel");
}

#[test]
fn reader_state_labels() {
    assert_eq!(ReaderState::Ok.title(), "● leyendo");
    assert_eq!(ReaderState::Idle.title(), "esperando a WoW");
    assert_eq!(ReaderState::NoStrip.title(), "sin franja");
    assert_eq!(ReaderState::Error.title(), "● detenido");
    assert_eq!(ReaderState::Starting.title(), "iniciando…");
    assert_eq!(ReaderState::Ok.footer(), "Lector activo");
    assert_eq!(ReaderState::NoStrip.footer(), "Sin franja del addon");
}

#[test]
fn translator_label_names_provider_and_model() {
    let mut s = TranslatorSettings::default();
    assert_eq!(translator_label(&s), "Google");
    s.provider = "openrouter".into();
    assert_eq!(translator_label(&s), "Google", "no model: Google is used");
    s.model = "gpt-x".into();
    assert_eq!(translator_label(&s), "OpenRouter · gpt-x");
}

// Debug log --------------------------------------------------------------------------

#[test]
fn iso_utc_formats_epoch_millis() {
    assert_eq!(iso_utc(1_793_967_690_123), "2026-11-06T12:21:30.123Z");
    assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
}

#[test]
fn debug_log_rotates_past_the_limit() {
    let dir = std::env::temp_dir().join(format!("wct-log-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("debug.log");
    let log = DebugLog::new(file.clone(), 200);
    for i in 0..20 {
        log.write(&format!("line {i} with some padding text"));
    }
    let old = dir.join("debug.log.old");
    assert!(old.exists(), "rotated file exists");
    let current = std::fs::read_to_string(&file).unwrap();
    assert!(current.len() < 200 + 80, "current file restarted after rotation");
    assert!(current.ends_with("line 19 with some padding text\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

// Autostart --------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn autostart_command_quotes_the_exe_and_starts_hidden() {
    use std::path::Path;
    use wow_chat_translator::app::autostart::command_line;
    assert_eq!(
        command_line(Path::new(r"C:\Program Files\WCT\wow-chat-translator.exe")),
        r#""C:\Program Files\WCT\wow-chat-translator.exe" --hidden"#
    );
}
