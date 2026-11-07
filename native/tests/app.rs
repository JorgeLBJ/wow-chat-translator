//! Pure logic behind the overlay window: presence gate, game watcher, message list
//! (unread counters, item cap, badges) and formatting helpers.

use wow_chat_translator::app::debug_log::{iso_utc, DebugLog};
use wow_chat_translator::app::game_watcher::{GameChange, GameWatcher};
use wow_chat_translator::app::messages::{badge, can_translate, flip_label, row_mode, shows_translation, BadgeKind, MessageList, Mode, Tab, MAX_ITEMS};
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
        links: Vec::new(),
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
fn registro_tab_comes_after_susurros_and_shows_log_items() {
    assert_eq!(Tab::ALL.last(), Some(&Tab::Log));
    assert_eq!(Tab::ALL[Tab::ALL.len() - 2], Tab::Whisper);
    assert_eq!(Tab::Log.label(), "Registro");
    assert_eq!(Tab::Log.key(), Some("log"));
    assert_eq!(Tab::from_key("log"), Some(Tab::Log));
    let mut list = MessageList::default();
    add(&mut list, 1, Some("trade"));
    add(&mut list, 2, Some("log"));
    assert_eq!(list.visible().count(), 2, "Todos includes log items");
    list.set_active(Tab::Log);
    let ids: Vec<u64> = list.visible().map(|r| r.item.id).collect();
    assert_eq!(ids, vec![2]);
    list.set_active(Tab::Trade);
    add(&mut list, 3, Some("log"));
    assert_eq!(list.unread_badge(Tab::Log), Some(1));
}

#[test]
fn dropping_the_log_removes_log_rows_and_leaves_registro() {
    let mut list = MessageList::default();
    add(&mut list, 1, Some("trade"));
    add(&mut list, 2, Some("log"));
    add(&mut list, 3, None);
    list.set_active(Tab::Log);
    list.drop_log();
    assert_eq!(list.active(), Tab::All);
    let ids: Vec<u64> = list.visible().map(|r| r.item.id).collect();
    assert_eq!(ids, vec![1, 3]);
    // Another active tab is kept.
    list.set_active(Tab::Trade);
    list.drop_log();
    assert_eq!(list.active(), Tab::Trade);
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
fn a_flipped_row_shows_the_other_text_and_its_menu_entry_names_it() {
    let mut it = item(1, None);
    // Nothing to flip without a translation (log lines, Spanish, not translated yet).
    assert_eq!(row_mode(Mode::Translated, true), Mode::Original);
    assert_eq!(flip_label(&it, Mode::Translated), None);
    it.translated = Some("hola".into());
    assert_eq!(row_mode(Mode::Translated, false), Mode::Translated);
    assert_eq!(row_mode(Mode::Original, true), Mode::Translated);
    assert_eq!(flip_label(&it, row_mode(Mode::Translated, false)), Some("Ver original"));
    assert_eq!(flip_label(&it, row_mode(Mode::Translated, true)), Some("Ver traducción"));
    assert_eq!(flip_label(&it, row_mode(Mode::Original, false)), Some("Ver traducción"));
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

// E5: settings form (translator:get/set/models/test semantics) and compose -----------

mod e5 {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wow_chat_translator::app::compose::{copied_message, failed_message, translate_outgoing};
    use wow_chat_translator::app::settings_form::{
        filter_models, form_for_provider, key_placeholder, load_models, models_message, probe, request_key, save_translator,
        saved_translator, validate_form, KeyInput, TranslatorForm, TEST_SAMPLE,
    };
    use wow_chat_translator::core::pipeline::{Translation, Translator};
    use wow_chat_translator::core::prefs::{Config, TranslatorSettings};
    use wow_chat_translator::secret::{encrypt_api_key, read_api_key};
    use wow_chat_translator::translators::key_binding::key_binding;
    use wow_chat_translator::translators::{Llm, ModelSource};

    const OR_URL: &str = "https://openrouter.ai/api/v1";

    fn form(provider: &str, url: &str, model: &str, key: KeyInput) -> TranslatorForm {
        TranslatorForm { provider: provider.into(), base_url: url.into(), model: model.into(), key }
    }

    fn saved(provider: &str, url: &str, enc: Option<&str>, binding: Option<String>) -> TranslatorSettings {
        TranslatorSettings {
            provider: provider.into(),
            base_url: url.into(),
            model: "m".into(),
            api_key_enc: enc.map(String::from),
            key_binding: binding,
        }
    }

    fn no_encrypt(_: &str) -> Result<String, String> {
        panic!("must not encrypt")
    }

    fn fake_encrypt(k: &str) -> Result<String, String> {
        Ok(format!("ENC({k})"))
    }

    // Save ---------------------------------------------------------------------------

    #[test]
    fn save_keeps_the_saved_key_when_untouched_and_the_binding_is_unchanged() {
        let prev = saved("openrouter", OR_URL, Some("E1"), Some(key_binding("openrouter", OR_URL)));
        let f = form("openrouter", " https://openrouter.ai/api/v1/ ", " gpt ", KeyInput::Untouched);
        let next = save_translator(&prev, &f, &no_encrypt).unwrap();
        assert_eq!(next.provider, "openrouter");
        assert_eq!(next.base_url, "https://openrouter.ai/api/v1/");
        assert_eq!(next.model, "gpt");
        assert_eq!(next.api_key_enc.as_deref(), Some("E1"));
        assert_eq!(next.key_binding, Some(key_binding("openrouter", OR_URL)));
    }

    #[test]
    fn save_drops_an_untouched_key_when_the_server_or_provider_changes() {
        let prev = saved("openrouter", OR_URL, Some("E1"), Some(key_binding("openrouter", OR_URL)));
        let other_url =
            save_translator(&prev, &form("openrouter", "https://evil.example/v1", "m", KeyInput::Untouched), &no_encrypt).unwrap();
        assert_eq!((other_url.api_key_enc, other_url.key_binding), (None, None));
        let other_provider = save_translator(&prev, &form("custom", OR_URL, "m", KeyInput::Untouched), &no_encrypt).unwrap();
        assert_eq!((other_provider.api_key_enc, other_provider.key_binding), (None, None));
    }

    #[test]
    fn save_deletes_the_key_when_the_field_is_emptied() {
        let prev = saved("openrouter", OR_URL, Some("E1"), Some(key_binding("openrouter", OR_URL)));
        for typed in ["", "   "] {
            let f = form("openrouter", OR_URL, "m", KeyInput::Typed(typed.into()));
            let next = save_translator(&prev, &f, &no_encrypt).unwrap();
            assert_eq!((next.api_key_enc, next.key_binding), (None, None), "{typed:?}");
        }
    }

    #[test]
    fn save_encrypts_a_new_key_and_binds_it_to_provider_and_server() {
        let prev = TranslatorSettings::default();
        let f = form("lmstudio", "", "qwen", KeyInput::Typed(" sk-new ".into()));
        let next = save_translator(&prev, &f, &fake_encrypt).unwrap();
        assert_eq!(next.api_key_enc.as_deref(), Some("ENC(sk-new)"));
        assert_eq!(next.key_binding, Some(key_binding("lmstudio", "http://localhost:1234/v1")));
    }

    #[test]
    fn save_refuses_anthropic_without_a_key_and_unknown_providers() {
        let prev = TranslatorSettings::default();
        let missing = Err("Pega tu clave API de Anthropic antes de guardar".to_string());
        assert_eq!(save_translator(&prev, &form("anthropic", "", "claude-haiku-5-5", KeyInput::Untouched), &no_encrypt), missing);
        assert_eq!(
            save_translator(&prev, &form("anthropic", "", "claude-haiku-5-5", KeyInput::Typed(" ".into())), &no_encrypt),
            missing
        );
        assert_eq!(
            save_translator(&prev, &form("deepl", "", "m", KeyInput::Untouched), &no_encrypt),
            Err("Proveedor desconocido".to_string())
        );
    }

    #[test]
    fn save_keeps_an_anthropic_key_whatever_the_url_field_holds() {
        let binding = key_binding("anthropic", "https://api.anthropic.com");
        let prev = saved("anthropic", "", Some("E2"), Some(binding.clone()));
        let f = form("anthropic", "https://evil.example", "claude-haiku-5-5", KeyInput::Untouched);
        let next = save_translator(&prev, &f, &no_encrypt).unwrap();
        assert_eq!(next.api_key_enc.as_deref(), Some("E2"));
        assert_eq!(next.key_binding, Some(binding));
    }

    #[test]
    fn save_reports_when_the_key_cannot_be_encrypted() {
        let fail = |_: &str| -> Result<String, String> { Err("x".into()) };
        let f = form("openrouter", "", "m", KeyInput::Typed("k".into()));
        assert_eq!(
            save_translator(&TranslatorSettings::default(), &f, &fail),
            Err("Windows no permite cifrar la clave; no se guardó".to_string())
        );
    }

    #[test]
    fn a_model_is_required_before_saving_an_llm() {
        assert_eq!(
            validate_form(&form("openrouter", "", " ", KeyInput::Untouched)),
            Err("Elige un modelo antes de guardar.".to_string())
        );
        assert_eq!(validate_form(&form("google", "", "", KeyInput::Untouched)), Ok(()));
        assert_eq!(validate_form(&form("lmstudio", "", "qwen", KeyInput::Untouched)), Ok(()));
    }

    // Form state (translator:get + syncProvider) -------------------------------------

    #[test]
    fn saved_translator_reports_a_key_only_when_its_binding_matches() {
        let mut config = Config::default();
        config.translator = saved("openrouter", "", Some("E1"), Some(key_binding("openrouter", OR_URL)));
        assert!(saved_translator(&config).has_key);
        config.translator.key_binding = Some(key_binding("openrouter", "https://other.example"));
        assert!(!saved_translator(&config).has_key);
        config.translator.api_key_enc = None;
        config.translator.key_binding = Some(key_binding("openrouter", OR_URL));
        assert!(!saved_translator(&config).has_key);
    }

    #[test]
    fn switching_provider_fills_presets_or_the_saved_values() {
        let mut config = Config::default();
        config.translator =
            saved("custom", "http://my.server/v1", Some("E"), Some(key_binding("custom", "http://my.server/v1")));
        config.translator.model = "my-model".into();
        let s = saved_translator(&config);
        assert_eq!(form_for_provider("custom", &s), form("custom", "http://my.server/v1", "my-model", KeyInput::Untouched));
        assert_eq!(form_for_provider("openrouter", &s), form("openrouter", OR_URL, "", KeyInput::Untouched));
        assert_eq!(form_for_provider("lmstudio", &s), form("lmstudio", "http://localhost:1234/v1", "", KeyInput::Untouched));
        assert_eq!(form_for_provider("anthropic", &s), form("anthropic", "", "claude-haiku-5-5", KeyInput::Untouched));
        assert_eq!(form_for_provider("claudecode", &s), form("claudecode", "", "claude-haiku-5-5", KeyInput::Untouched));
        assert_eq!(key_placeholder("custom", &s), "•••••• guardada (escribe para cambiarla)");
        assert_eq!(key_placeholder("openrouter", &s), "sin clave");
    }

    // Which key Probar / Cargar modelos may use ----------------------------------------

    #[test]
    fn an_untouched_key_is_read_only_through_the_binding_check() {
        let config = Config::default();
        let calls = RefCell::new(Vec::new());
        let read = |_: &Config, p: &str, u: &str| {
            calls.borrow_mut().push((p.to_string(), u.to_string()));
            Some("from-read".to_string())
        };
        let key = request_key(&config, &form("openrouter", " https://x.example/v1 ", "m", KeyInput::Untouched), &read);
        assert_eq!(key.as_deref(), Some("from-read"));
        assert_eq!(*calls.borrow(), vec![("openrouter".to_string(), "https://x.example/v1".to_string())]);
    }

    #[test]
    fn a_typed_key_is_used_as_typed_and_claude_code_never_gets_one() {
        let read = |_: &Config, _: &str, _: &str| -> Option<String> { panic!("must not read the saved key") };
        let config = Config::default();
        let typed = |s: &str| KeyInput::Typed(s.into());
        assert_eq!(request_key(&config, &form("openrouter", "", "m", typed(" sk-1 ")), &read).as_deref(), Some("sk-1"));
        assert_eq!(request_key(&config, &form("openrouter", "", "m", typed("  ")), &read), None);
        assert_eq!(request_key(&config, &form("claudecode", "", "m", KeyInput::Untouched), &read), None);
        assert_eq!(request_key(&config, &form("claudecode", "", "m", typed("k")), &read), None);
    }

    #[test]
    fn with_the_real_reader_a_saved_key_never_reaches_another_server() {
        let mut config = Config::default();
        let enc = encrypt_api_key("sk-or-9").unwrap();
        config.translator = saved("openrouter", "", Some(&enc), Some(key_binding("openrouter", OR_URL)));
        let k = |p: &str, u: &str| request_key(&config, &form(p, u, "m", KeyInput::Untouched), &read_api_key);
        assert_eq!(k("openrouter", OR_URL).as_deref(), Some("sk-or-9"));
        assert_eq!(k("openrouter", "https://evil.example/api/v1"), None);
        assert_eq!(k("custom", OR_URL), None);
        assert_eq!(k("anthropic", ""), None);
    }

    // Probar / Cargar modelos -------------------------------------------------------------

    fn t(text: &str) -> Translation {
        Translation { text: text.into(), source: "en".into(), provider: None, fallback_reason: None }
    }

    struct FakeLlm {
        reply: Result<String, String>,
        models: Vec<String>,
    }
    impl Translator for FakeLlm {
        fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
            assert_eq!((text, target), (TEST_SAMPLE, "es"));
            self.reply.clone().map(|s| t(&s))
        }
    }
    impl ModelSource for FakeLlm {
        fn list_models(&self) -> Result<Vec<String>, String> {
            Ok(self.models.clone())
        }
    }

    struct Google;
    impl Translator for Google {
        fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
            assert_eq!((text, target), (TEST_SAMPLE, "es"));
            Ok(t("LFM ZG necesito curar, susurra"))
        }
    }

    type MakeLlm<'a> = &'a dyn Fn(&TranslatorSettings, Option<String>) -> Result<Box<dyn Llm>, String>;

    #[test]
    fn probe_translates_the_sample_with_google_or_the_llm() {
        assert_eq!(TEST_SAMPLE, "LFM ZG need heal, pst");
        let never: MakeLlm = &|_, _| panic!("no llm for google");
        assert_eq!(
            probe(&form("google", "", "", KeyInput::Untouched), None, &Google, never),
            Ok("«LFM ZG need heal, pst» → «LFM ZG necesito curar, susurra»".to_string())
        );
        assert_eq!(probe(&form("openrouter", "", "", KeyInput::Untouched), None, &Google, never), Err("Elige un modelo".to_string()));

        let seen = RefCell::new(None);
        let make = |s: &TranslatorSettings, key: Option<String>| -> Result<Box<dyn Llm>, String> {
            *seen.borrow_mut() = Some((s.provider.clone(), s.base_url.clone(), s.model.clone(), key));
            Ok(Box::new(FakeLlm { reply: Ok("hola".into()), models: vec![] }))
        };
        let f = form("lmstudio", " http://localhost:1234/v1 ", " qwen ", KeyInput::Untouched);
        assert_eq!(probe(&f, Some("k".into()), &Google, &make), Ok("«LFM ZG need heal, pst» → «hola»".to_string()));
        assert_eq!(
            *seen.borrow(),
            Some(("lmstudio".into(), "http://localhost:1234/v1".into(), "qwen".into(), Some("k".into())))
        );
        let failing: MakeLlm = &|_, _| Err("Falta la URL del servidor".into());
        assert_eq!(
            probe(&form("custom", "", "m", KeyInput::Untouched), None, &Google, failing),
            Err("Falta la URL del servidor".to_string())
        );
    }

    #[test]
    fn load_models_lists_the_server_models() {
        let make: MakeLlm = &|_, _| Ok(Box::new(FakeLlm { reply: Err("x".into()), models: vec!["only".into()] }));
        let models = load_models(&form("lmstudio", "", "", KeyInput::Untouched), None, make).unwrap();
        assert_eq!(models, vec!["only".to_string()]);
        assert_eq!(models_message(3), "3 modelos disponibles. Elige uno en la lista ▾ junto al campo Modelo.");
    }

    #[test]
    fn filter_models_matches_every_word_ignoring_case_and_shows_all_for_a_chosen_model() {
        let models: Vec<String> =
            ["deepseek/deepseek-chat", "deepseek/deepseek-r1", "openai/gpt-5-mini", "DeepSeek/V3 Chat"].map(String::from).to_vec();
        let all: Vec<&str> = models.iter().map(String::as_str).collect();
        assert_eq!(filter_models(&models, ""), all);
        assert_eq!(filter_models(&models, "  "), all);
        assert_eq!(filter_models(&models, "DEEPSEEK"), vec!["deepseek/deepseek-chat", "deepseek/deepseek-r1", "DeepSeek/V3 Chat"]);
        assert_eq!(filter_models(&models, "deep chat"), vec!["deepseek/deepseek-chat", "DeepSeek/V3 Chat"]);
        assert_eq!(filter_models(&models, "claude"), Vec::<&str>::new());
        // The field already holds a model from the list: show them all to switch.
        assert_eq!(filter_models(&models, "openai/gpt-5-mini"), all);
    }

    // Compose ------------------------------------------------------------------------------

    struct Counting(AtomicUsize, Result<String, String>);
    impl Translator for Counting {
        fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            assert_eq!((text, target), ("hola a todos", "en"));
            self.1.clone().map(|s| t(&s))
        }
    }

    #[test]
    fn compose_passes_spanish_through_without_translating() {
        let tr = Counting(AtomicUsize::new(0), Ok("x".into()));
        assert_eq!(translate_outgoing("  hola a todos ", "es", &tr), Ok("hola a todos".to_string()));
        assert_eq!(tr.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn compose_translates_to_english_and_reports_errors() {
        let tr = Counting(AtomicUsize::new(0), Ok("hello everyone".into()));
        assert_eq!(translate_outgoing("hola a todos\n", "en", &tr), Ok("hello everyone".to_string()));
        assert_eq!(tr.0.load(Ordering::SeqCst), 1);
        let bad = Counting(AtomicUsize::new(0), Err("Google: HTTP 429".into()));
        assert_eq!(translate_outgoing("hola a todos", "en", &bad), Err("Google: HTTP 429".to_string()));
        assert_eq!(translate_outgoing("   ", "en", &bad), Err("Mensaje vacío".to_string()));
        assert_eq!(bad.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn compose_messages_match_the_electron_overlay() {
        assert_eq!(copied_message("hello"), "✓ Copiado: «hello». En WoW: Enter → Ctrl+V → Enter");
        assert_eq!(failed_message("sin red"), "No se pudo traducir: sin red");
    }
}
