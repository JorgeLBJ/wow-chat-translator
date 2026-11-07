// Ported from app/test/core.test.js (links, emoji, looksSpanish, channelTab, message
// pipeline with exact strip text) plus prefs/config.json compatibility.

use std::cell::{Cell, RefCell};

use wow_chat_translator::core::channels::{channel_tab, is_log_channel, log_label};
use wow_chat_translator::core::emoji::replace_shortcodes;
use wow_chat_translator::core::language::looks_spanish;
use wow_chat_translator::core::links::{marked_link_refs, only_links, protect_marked_links, restore_links, strip_link_markers, LinkRef};
use wow_chat_translator::core::pipeline::{
    filter_log_messages, ItemPatch, Job, OverlayItem, Pipeline, Status, Translation, Translator, UiEvent,
};
use wow_chat_translator::core::prefs::{self, Bounds, Config};
use wow_chat_translator::strip::Message;

// ---------------------------------------------------------------- links

#[test]
fn marked_links_become_placeholders_and_are_restored() {
    let (text, links) = protect_marked_links("wts \u{2}[Arcanite Bar]\u{3} and \u{2}[Small Black Pouch]\u{3} cheap");
    assert_eq!(text, "wts [0] and [1] cheap");
    assert_eq!(links, vec!["[Arcanite Bar]".to_string(), "[Small Black Pouch]".to_string()]);
    assert_eq!(
        restore_links("vendo [0] y [ 1 ] barato", &links),
        "vendo [Arcanite Bar] y [Small Black Pouch] barato"
    );
}

#[test]
fn typed_brackets_are_not_links() {
    let (text, links) = protect_marked_links("wts \u{2}[Arcanite Bar]\u{3} and [cheap stuff] pst");
    assert_eq!(text, "wts [0] and [cheap stuff] pst");
    assert_eq!(links, vec!["[Arcanite Bar]".to_string()]);
}

#[test]
fn stray_markers_are_removed() {
    assert_eq!(strip_link_markers("a\u{2}[X]\u{3} b \u{3}c\u{2}"), "a[X] b c");
    let (text, links) = protect_marked_links("x \u{2}y \u{2}[Z]\u{3} \u{3}");
    assert_eq!(text, "x y [0] ");
    assert_eq!(links, vec!["[Z]".to_string()]);
}

#[test]
fn unknown_placeholder_numbers_are_left_alone() {
    assert_eq!(restore_links("tengo [5]", &["[Bar]".to_string()]), "tengo [5]");
    assert_eq!(restore_links("[[0]]", &["[Bar]".to_string()]), "[[Bar]]");
}

#[test]
fn detects_a_message_that_is_only_links() {
    assert!(only_links("[0]"));
    assert!(only_links(" [0] [1] ! "));
    assert!(!only_links("wts [0]"));
}

// ---------------------------------------------------------------- emoji

#[test]
fn replaces_known_shortcodes() {
    for (input, expected) in [
        (":sob:", "😭"),
        ("gg :joy: :fire:", "gg 😂 🔥"),
        ("that's :100:", "that's 💯"),
        ("nice :+1:", "nice 👍"),
        (":SOB:", "😭"),
        (":skull:", "💀"),
    ] {
        assert_eq!(replace_shortcodes(input), expected, "{input}");
    }
}

#[test]
fn leaves_non_shortcodes_unchanged() {
    for text in ["meet at 12:30:45", ":notanemoji:", "ratio 3:2:1", "WTS [Recipe: Elixir]", "a:sob:", ":sob:b"] {
        assert_eq!(replace_shortcodes(text), text, "{text}");
    }
}

// ---------------------------------------------------------------- language

#[test]
fn detects_spanish() {
    for text in [
        "alguien para Deadmines?",
        "¿alguien sabe dónde está el instructor?",
        "necesito un tanque para la mazmorra",
    ] {
        assert!(looks_spanish(text), "{text}");
    }
}

#[test]
fn does_not_flag_non_spanish() {
    for text in [
        "anyone know where the trainer is?",
        "bora fazer raid hoje à noite",
        "ich brauche noch Mana, kurz warten",
        "LFM ZG need heal",
    ] {
        assert!(!looks_spanish(text), "{text}");
    }
}

// ---------------------------------------------------------------- channels

#[test]
fn channel_tab_table() {
    for (channel, tab) in [
        ("General", Some("general")),
        ("Trade", Some("trade")),
        ("Comercio", Some("trade")),
        ("Guild", Some("guild")),
        ("Officer", Some("guild")),
        ("Party", Some("group")),
        ("Raid", Some("group")),
        ("Instance", Some("group")),
        ("Whisper", Some("whisper")),
        ("Say", None),
        ("LookingForGroup", None),
    ] {
        assert_eq!(channel_tab(channel), tab, "{channel}");
    }
}

// ---------------------------------------------------------------- pipeline

/// Records calls; answers ES(text) from "en" unless told to fail.
struct FakeTranslator {
    calls: RefCell<Vec<String>>,
    fail: Cell<bool>,
    echo: bool,
}

impl FakeTranslator {
    fn new() -> Self {
        Self { calls: RefCell::new(Vec::new()), fail: Cell::new(false), echo: false }
    }
}

impl Translator for FakeTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        assert_eq!(target, "es");
        self.calls.borrow_mut().push(text.to_string());
        if self.fail.get() {
            return Err("down".into());
        }
        Ok(Translation {
            text: if self.echo { text.to_string() } else { format!("ES({text})") },
            source: "en".into(),
            provider: Some("X".into()),
            fallback_reason: None,
        })
    }
}

fn msg(channel: &str, text: &str) -> Message {
    Message { channel: channel.into(), class: Some("MAGE".into()), player: "Ana".into(), text: text.into() }
}

fn pipeline(paused: bool) -> Pipeline {
    let mut p = Pipeline::new("es");
    p.set_clock(Box::new(|| 1234));
    p.set_paused(paused);
    p
}

/// Runs every queued job through the translator, like a worker thread would.
fn run(p: &mut Pipeline, tr: &dyn Translator) {
    for Job { id, text, target } in p.take_jobs() {
        let res = tr.translate(&text, &target);
        p.complete(id, res);
    }
}

fn added(events: &[UiEvent]) -> Vec<&OverlayItem> {
    events.iter().filter_map(|e| if let UiEvent::Add(i) = e { Some(i) } else { None }).collect()
}

fn updates(events: &[UiEvent]) -> Vec<(u64, &ItemPatch)> {
    events.iter().filter_map(|e| if let UiEvent::Update(id, p) = e { Some((*id, p)) } else { None }).collect()
}

#[test]
fn translates_new_messages_and_keeps_links_untranslated() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(false);
    p.add_messages(&[msg("Trade", "wts \u{2}[Arcanite Bar]\u{3} cheap")]);
    run(&mut p, &tr);
    let ev = p.take_events();
    assert_eq!(ev.len(), 2);
    let UiEvent::Add(item) = &ev[0] else { panic!("add first") };
    assert_eq!(item.id, 1);
    assert_eq!(item.time, 1234);
    assert_eq!(item.status, Status::Pending);
    assert_eq!(item.original, "wts [Arcanite Bar] cheap");
    assert_eq!(item.channel, "Trade");
    assert_eq!(item.tab, Some("trade"));
    assert_eq!(item.class.as_deref(), Some("MAGE"));
    assert_eq!(item.player, "Ana");
    assert_eq!(*tr.calls.borrow(), vec!["wts [0] cheap".to_string()]);
    let UiEvent::Update(1, patch) = &ev[1] else { panic!("update second") };
    assert_eq!(patch.status, Status::Done);
    assert_eq!(patch.translated.as_deref(), Some("ES(wts [Arcanite Bar] cheap)"));
    assert_eq!(patch.lang.as_deref(), Some("en"));
    assert_eq!(patch.provider.as_deref(), Some("X"));
}

#[test]
fn does_not_translate_while_paused() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(true);
    p.add_messages(&[msg("Trade", "anyone know where the trainer is?")]);
    run(&mut p, &tr);
    assert!(tr.calls.borrow().is_empty());
    let ev = p.take_events();
    assert_eq!(ev.len(), 1);
    assert_eq!(added(&ev)[0].status, Status::Paused);
}

#[test]
fn translates_one_paused_message_on_demand() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(true);
    p.add_messages(&[msg("Trade", "need a tank"), msg("Trade", "lf healer")]);
    assert!(p.translate_by_id(2));
    run(&mut p, &tr);
    assert_eq!(*tr.calls.borrow(), vec!["lf healer".to_string()]);
    let ev = p.take_events();
    let up = updates(&ev);
    assert_eq!(up.len(), 2);
    assert_eq!(up[0], (2, &ItemPatch::status(Status::Pending)));
    assert_eq!(up[1].0, 2);
    assert_eq!(up[1].1.status, Status::Done);
    assert_eq!(up[1].1.translated.as_deref(), Some("ES(lf healer)"));
}

#[test]
fn only_translates_messages_that_arrive_after_resuming() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(true);
    p.add_messages(&[msg("Trade", "need a tank")]);
    p.set_paused(false);
    p.add_messages(&[msg("Trade", "lf healer")]);
    run(&mut p, &tr);
    assert_eq!(*tr.calls.borrow(), vec!["lf healer".to_string()]);
}

#[test]
fn spanish_and_link_only_messages_are_not_translated_paused_or_not() {
    for paused in [true, false] {
        let tr = FakeTranslator::new();
        let mut p = pipeline(paused);
        p.add_messages(&[msg("Trade", "alguien para la mazmorra?"), msg("Trade", "\u{2}[Linen Cloth]\u{3}")]);
        run(&mut p, &tr);
        assert!(tr.calls.borrow().is_empty());
        let ev = p.take_events();
        let items = added(&ev);
        assert_eq!(items.len(), 2);
        for item in &items {
            assert_eq!(item.status, Status::Done);
            assert_eq!(item.lang.as_deref(), Some("es"));
            assert_eq!(item.translated, None);
        }
        assert_eq!(items[1].original, "[Linen Cloth]");
        assert!(!p.translate_by_id(1));
        assert!(!p.translate_by_id(2));
    }
}

#[test]
fn a_failed_translation_can_be_retried_on_demand() {
    let tr = FakeTranslator::new();
    tr.fail.set(true);
    let mut p = pipeline(false);
    p.add_messages(&[msg("Say", "hello there")]);
    run(&mut p, &tr);
    let ev = p.take_events();
    let up = updates(&ev);
    assert_eq!(up.last().unwrap(), &(1, &ItemPatch::error("down")));
    tr.fail.set(false);
    assert!(p.translate_by_id(1));
    run(&mut p, &tr);
    let ev = p.take_events();
    let (id, patch) = *updates(&ev).last().unwrap();
    assert_eq!(id, 1);
    assert_eq!(patch.status, Status::Done);
    assert_eq!(patch.translated.as_deref(), Some("ES(hello there)"));
}

#[test]
fn returns_false_for_unknown_or_already_translated_messages() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(false);
    p.add_messages(&[msg("Say", "hello there")]);
    run(&mut p, &tr);
    assert!(!p.translate_by_id(1));
    assert!(!p.translate_by_id(99));
}

#[test]
fn ignores_a_second_request_while_in_flight() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(true);
    p.add_messages(&[msg("Say", "hello there")]);
    assert!(p.translate_by_id(1));
    assert!(!p.translate_by_id(1));
    run(&mut p, &tr);
    assert_eq!(tr.calls.borrow().len(), 1);
    // A job taken but not completed yet is still in flight.
    let mut p = pipeline(true);
    p.add_messages(&[msg("Say", "hello there")]);
    assert!(p.translate_by_id(1));
    let jobs = p.take_jobs();
    assert_eq!(jobs.len(), 1);
    assert!(!p.translate_by_id(1));
}

#[test]
fn exact_strip_text_is_kept_as_is() {
    let mut p = pipeline(false);
    p.add_messages(&[msg("Trade", "101"), msg("Say", "i < 3 > you")]);
    let ev = p.take_events();
    let originals: Vec<&str> = added(&ev).iter().map(|i| i.original.as_str()).collect();
    assert_eq!(originals, vec!["101", "i < 3 > you"]);
    // "101" has no letters, so like the JS pipeline it is shown as is, never translated.
    let jobs = p.take_jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].text, "i < 3 > you");
}

#[test]
fn same_text_or_target_source_means_no_translation() {
    let mut tr = FakeTranslator::new();
    tr.echo = true;
    let mut p = pipeline(false);
    p.add_messages(&[msg("Say", "Hello There ")]);
    run(&mut p, &tr);
    let ev = p.take_events();
    let (_, patch) = updates(&ev)[0];
    assert_eq!(patch.status, Status::Done);
    assert_eq!(patch.translated, None);

    let mut p = pipeline(false);
    p.add_messages(&[msg("Say", "hello there")]);
    let job = p.take_jobs().remove(0);
    p.complete(job.id, Ok(Translation { text: "hola".into(), source: "es".into(), provider: None, fallback_reason: None }));
    let ev = p.take_events();
    assert_eq!(updates(&ev)[0].1.translated, None);
    assert_eq!(updates(&ev)[0].1.lang.as_deref(), Some("es"));

    let mut p = pipeline(false);
    p.add_messages(&[msg("Say", "hello there")]);
    let job = p.take_jobs().remove(0);
    p.complete(
        job.id,
        Ok(Translation { text: "hola".into(), source: "auto".into(), provider: Some("google".into()), fallback_reason: Some("timeout".into()) }),
    );
    let ev = p.take_events();
    let patch = updates(&ev)[0].1;
    assert_eq!(patch.lang, None);
    assert_eq!(patch.translated.as_deref(), Some("hola"));
    assert_eq!(patch.fallback_reason.as_deref(), Some("timeout"));
}

#[test]
fn shortcodes_are_replaced_before_translating() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(false);
    p.add_messages(&[msg("Say", "i died again :sob:")]);
    run(&mut p, &tr);
    let ev = p.take_events();
    assert_eq!(added(&ev)[0].original, "i died again 😭");
    assert_eq!(tr.calls.borrow()[0], "i died again 😭");
}

#[test]
fn keeps_at_most_300_pending_entries() {
    let mut p = pipeline(true);
    let msgs: Vec<Message> = (0..301).map(|i| msg("Say", &format!("hello there {i}"))).collect();
    p.add_messages(&msgs);
    assert!(!p.translate_by_id(1), "oldest entry evicted");
    assert!(p.translate_by_id(2));
    assert!(p.translate_by_id(301));
}

#[test]
fn applying_patches_updates_an_item() {
    let mut p = pipeline(false);
    p.add_messages(&[msg("Say", "hello there")]);
    let ev = p.take_events();
    let mut item = added(&ev)[0].clone();
    let job = p.take_jobs().remove(0);
    p.complete(job.id, Err("down".into()));
    for e in p.take_events() {
        if let UiEvent::Update(_, patch) = e {
            item.apply(&patch);
        }
    }
    assert_eq!(item.status, Status::Error);
    assert_eq!(item.error.as_deref(), Some("down"));
    assert!(p.translate_by_id(1));
    let tr = FakeTranslator::new();
    run(&mut p, &tr);
    for e in p.take_events() {
        if let UiEvent::Update(_, patch) = e {
            item.apply(&patch);
        }
    }
    assert_eq!(item.status, Status::Done);
    assert_eq!(item.error, None);
    assert_eq!(item.translated.as_deref(), Some("ES(hello there)"));
    assert_eq!(item.lang.as_deref(), Some("en"));
    assert_eq!(item.provider.as_deref(), Some("X"));
}

// ---------------------------------------------------------------- prefs

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wct-core-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn defaults_when_file_is_missing() {
    let dir = scratch("missing");
    let c = prefs::load(&dir.join("config.json"));
    assert_eq!(c, Config::default());
    assert_eq!(c.prefs.font_size, 13.0);
    assert_eq!(c.prefs.opacity, 1.0);
    assert!(c.prefs.always_on_top);
    assert!(c.prefs.auto_start);
    assert!(!c.prefs.translation_paused);
    assert_eq!(c.translator.provider, "google");
    assert_eq!(c.translator.base_url, "");
    assert_eq!(c.translator.model, "");
    assert_eq!(c.translator.api_key_enc, None);
    assert_eq!(c.translator.key_binding, None);
    assert_eq!(c.overlay_bounds, None);
}

#[test]
fn config_round_trips() {
    let dir = scratch("roundtrip");
    let path = dir.join("nested").join("config.json");
    let mut c = Config::default();
    c.prefs.font_size = 16.0;
    c.prefs.translation_paused = true;
    c.translator.provider = "openrouter".into();
    c.translator.api_key_enc = Some("abc=".into());
    c.translator.key_binding = Some("openrouter|https://openrouter.ai/api/v1".into());
    c.overlay_bounds = Some(Bounds { x: Some(10), y: Some(-20), width: Some(560), height: Some(420) });
    prefs::save(&path, &c).unwrap();
    assert_eq!(prefs::load(&path), c);
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"translationPaused\": true"), "{raw}");
    assert!(raw.contains("\"apiKeyEnc\""), "{raw}");
    assert!(raw.contains("\"overlayBounds\""), "{raw}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reads_electron_config_and_ignores_unknown_keys() {
    let dir = scratch("electron");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(
        &path,
        r#"{
  "calibration": { "x": 1, "y": 2 },
  "prefs": { "fontSize": 15, "opacity": 0.85, "autoStart": false, "somethingNew": 1 },
  "translator": {
    "provider": "lmstudio",
    "baseUrl": "http://localhost:1234/v1",
    "model": "gemma",
    "apiKeyEnc": "djEwAAAA",
    "keyBinding": "lmstudio|http://localhost:1234/v1"
  },
  "overlayBounds": { "x": 1300, "y": 40, "width": 560, "height": 420 }
}"#,
    )
    .unwrap();
    let c = prefs::load(&path);
    assert_eq!(c.prefs.font_size, 15.0);
    assert_eq!(c.prefs.opacity, 0.85);
    assert!(!c.prefs.auto_start);
    assert!(c.prefs.always_on_top, "missing pref keeps its default");
    assert!(!c.prefs.translation_paused);
    assert_eq!(c.translator.provider, "lmstudio");
    assert_eq!(c.translator.base_url, "http://localhost:1234/v1");
    assert_eq!(c.translator.model, "gemma");
    assert_eq!(c.translator.api_key_enc.as_deref(), Some("djEwAAAA"));
    assert_eq!(c.translator.key_binding.as_deref(), Some("lmstudio|http://localhost:1234/v1"));
    assert_eq!(c.overlay_bounds, Some(Bounds { x: Some(1300), y: Some(40), width: Some(560), height: Some(420) }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unreadable_config_falls_back_to_defaults() {
    let dir = scratch("broken");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, "{ not json").unwrap();
    assert_eq!(prefs::load(&path), Config::default());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hide_from_capture_defaults_off_and_persists() {
    assert!(!Config::default().prefs.hide_from_capture);
    let dir = scratch("capture");
    let path = dir.join("config.json");
    let mut c = Config::default();
    c.prefs.hide_from_capture = true;
    prefs::save(&path, &c).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"hideFromCapture\": true"), "{raw}");
    assert!(prefs::load(&path).prefs.hide_from_capture);
    std::fs::write(&path, r#"{"prefs":{"fontSize":14}}"#).unwrap();
    assert!(!prefs::load(&path).prefs.hide_from_capture, "missing key means off");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- chat log (Registro) and link refs

const REF_LINK: &str = "wts \u{2}[Arcanite Bar]\u{4}item:12360\u{3} cheap";

fn log_msg(channel: &str, text: &str) -> Message {
    Message { channel: channel.into(), class: None, player: String::new(), text: text.into() }
}

#[test]
fn link_refs_are_never_shown() {
    assert_eq!(strip_link_markers(REF_LINK), "wts [Arcanite Bar] cheap");
    let (masked, links) = protect_marked_links(REF_LINK);
    assert_eq!(masked, "wts [0] cheap");
    assert_eq!(links, vec!["[Arcanite Bar]".to_string()]);
    assert_eq!(restore_links("vendo [0] barato", &links), "vendo [Arcanite Bar] barato");
}

#[test]
fn old_link_form_still_decodes_and_has_no_ref() {
    let old = "wts \u{2}[Arcanite Bar]\u{3} cheap";
    assert_eq!(strip_link_markers(old), "wts [Arcanite Bar] cheap");
    assert_eq!(protect_marked_links(old), ("wts [0] cheap".to_string(), vec!["[Arcanite Bar]".to_string()]));
    assert_eq!(marked_link_refs(old), Vec::<LinkRef>::new());
}

#[test]
fn link_refs_expose_type_and_id() {
    let text = "\u{2}[Linen Cloth]\u{4}item:2589\u{3} and \u{2}[Old]\u{3} and \u{2}[Quest]\u{4}quest:42\u{3}";
    assert_eq!(
        marked_link_refs(text),
        vec![
            LinkRef { name: "[Linen Cloth]".into(), kind: "item".into(), id: 2589 },
            LinkRef { name: "[Quest]".into(), kind: "quest".into(), id: 42 },
        ]
    );
    // A malformed ref is ignored but never shown.
    let bad = "\u{2}[X]\u{4}item:abc\u{3}";
    assert_eq!(marked_link_refs(bad), Vec::<LinkRef>::new());
    assert_eq!(strip_link_markers(bad), "[X]");
}

#[test]
fn log_channels_map_to_the_log_tab() {
    for channel in ["*Loot", "*Money", "*Currency", "*System", "*XP", "*Reputation", "*Skill", "*Achievement", "*NPC", "*BossEmote", "*Emote"] {
        assert!(is_log_channel(channel), "{channel}");
        assert_eq!(channel_tab(channel), Some("log"), "{channel}");
        assert!(log_label(channel).is_some(), "{channel}");
    }
    assert_eq!(log_label("*Loot"), Some("Botín"));
    assert_eq!(log_label("Trade"), None);
    // A player-made channel with a log-like name is player chat (only the addon
    // prefixes log labels with "*", which channel names cannot contain).
    for custom in ["Loot", "System", "XP", "NPC"] {
        assert!(!is_log_channel(custom), "{custom}");
        assert_eq!(channel_tab(custom), None, "{custom}");
    }
    assert!(!is_log_channel("Trade"));
    assert!(!is_log_channel("Say"));
}

#[test]
fn log_messages_are_done_untranslated_and_make_no_job() {
    for paused in [false, true] {
        let mut p = pipeline(paused);
        p.add_messages(&[log_msg("*Loot", "You receive loot: \u{2}[Linen Cloth]\u{4}item:2589\u{3}.")]);
        assert!(p.take_jobs().is_empty());
        let ev = p.take_events();
        assert_eq!(ev.len(), 1);
        let UiEvent::Add(item) = &ev[0] else { panic!("add") };
        assert_eq!(item.status, Status::Done);
        assert_eq!(item.tab, Some("log"));
        assert_eq!(item.player, "");
        assert_eq!(item.class, None);
        assert_eq!(item.translated, None);
        assert_eq!(item.lang, None);
        assert_eq!(item.original, "You receive loot: [Linen Cloth].");
        assert_eq!(item.links, vec![LinkRef { name: "[Linen Cloth]".into(), kind: "item".into(), id: 2589 }]);
        assert!(!p.translate_by_id(item.id), "log items are never translated on demand");
    }
}

#[test]
fn chat_with_a_link_ref_still_translates() {
    let tr = FakeTranslator::new();
    let mut p = pipeline(false);
    p.add_messages(&[msg("Trade", REF_LINK)]);
    run(&mut p, &tr);
    assert_eq!(*tr.calls.borrow(), vec!["wts [0] cheap".to_string()]);
    let ev = p.take_events();
    let UiEvent::Add(item) = &ev[0] else { panic!("add") };
    assert_eq!(item.tab, Some("trade"));
    assert_eq!(item.original, "wts [Arcanite Bar] cheap");
    assert_eq!(item.links, vec![LinkRef { name: "[Arcanite Bar]".into(), kind: "item".into(), id: 12360 }]);
    let UiEvent::Update(_, patch) = &ev[1] else { panic!("update") };
    assert_eq!(patch.translated.as_deref(), Some("ES(wts [Arcanite Bar] cheap)"));
}

#[test]
fn log_messages_are_dropped_when_the_log_is_off() {
    let all = vec![msg("Trade", "wts ore"), log_msg("*System", "You are now AFK."), msg("Say", "hi")];
    let off = filter_log_messages(&all, false);
    assert_eq!(off, vec![all[0].clone(), all[2].clone()]);
    assert_eq!(filter_log_messages(&all, true), all);
}

#[test]
fn show_log_defaults_off_and_persists() {
    assert!(!Config::default().prefs.show_log);
    let dir = scratch("showlog");
    let path = dir.join("config.json");
    let mut c = Config::default();
    c.prefs.show_log = true;
    prefs::save(&path, &c).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"showLog\": true"), "{raw}");
    assert!(prefs::load(&path).prefs.show_log);
    std::fs::write(&path, r#"{"prefs":{"fontSize":14}}"#).unwrap();
    assert!(!prefs::load(&path).prefs.show_log, "missing key means off");
    let _ = std::fs::remove_dir_all(&dir);
}
