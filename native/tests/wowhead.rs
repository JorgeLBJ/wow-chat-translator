// Item tooltips, "Wowhead (internet)" source: dataEnv mapping, URL, HTML -> lines,
// fetch outcomes with a fake HTTP client, the result cache and the panel choice.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use wow_chat_translator::core::item_tips::{ItemTooltip, TipLine, TOOLTIP_CACHE_CAP};
use wow_chat_translator::core::prefs::ItemTooltips;
use wow_chat_translator::core::wowhead::{
    data_env, decode_entities, env, fetch, html_to_lines, panel_view, parse_answer, tooltip_url, user_agent, PanelView,
    WowheadCache, WowheadState, LOADING_TEXT, MAX_IN_FLIGHT, NO_WOWHEAD_TEXT, RETRY_AFTER, TIMEOUT,
};
use wow_chat_translator::translators::http::{HttpClient, HttpResponse};

fn line(left: &str, right: Option<&str>) -> TipLine {
    TipLine { left: left.into(), right: right.map(str::to_string) }
}

fn tip(name: &str) -> ItemTooltip {
    ItemTooltip { quality: Some(4), lines: vec![line(name, None)] }
}

// ---------------------------------------------------------------- dataEnv and URL

#[test]
fn data_env_follows_the_played_flavor() {
    assert_eq!(data_env(Some("_retail_"), "Retail"), env::RETAIL);
    assert_eq!(env::RETAIL, 1);
    assert_eq!(data_env(Some("_ptr_"), "Retail PTR"), env::PTR);
    assert_eq!(data_env(Some("_beta_"), "Beta"), env::BETA);
    assert_eq!(data_env(Some("_classic_era_"), "Classic Era"), env::CLASSIC_ERA);
    assert_eq!(env::CLASSIC_ERA, 4);
    assert_eq!(data_env(Some("_classic_"), "Mists of Pandaria Classic"), env::MISTS);
    assert_eq!(env::MISTS, 15);
    assert_eq!(data_env(Some("_classic_"), "Cataclysm Classic"), env::CATA);
    assert_eq!(data_env(Some("_classic_"), "Wrath Classic"), env::WRATH);
    assert_eq!(data_env(Some("_classic_"), "Burning Crusade Classic"), env::TBC);
    assert_eq!(data_env(Some("_classic_"), "Classic"), env::MISTS, "unknown classic version: the live progression");
    assert_eq!(data_env(Some("_classic_ptr_"), "Mists of Pandaria Classic PTR"), env::MISTS);
    assert_eq!(data_env(Some("_forever_"), "WoW Forever"), env::FOREVER);
    assert_eq!(env::FOREVER, 16);
    assert_eq!(data_env(Some("_anniversary_"), "Anniversary"), env::TBC);
    assert_eq!(data_env(Some("_RETAIL_"), ""), env::RETAIL, "case-insensitive");
    assert_eq!(data_env(Some("_something_new_"), "x"), env::RETAIL);
    assert_eq!(data_env(None, ""), env::RETAIL, "no flavor known: retail");
}

#[test]
fn tooltip_url_carries_only_id_env_and_locale() {
    assert_eq!(tooltip_url(19019, 4, 6), "https://nether.wowhead.com/tooltip/item/19019?dataEnv=4&locale=6");
    assert!(user_agent().starts_with("WoWChatTranslator/"), "{}", user_agent());
    assert_eq!(user_agent(), format!("WoWChatTranslator/{}", env!("CARGO_PKG_VERSION")));
}

// ---------------------------------------------------------------- HTML -> lines

#[test]
fn html_entities_are_decoded() {
    assert_eq!(decode_entities("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#39; &#x41;"), "a & b <c> \"d\" 'e' A");
    assert_eq!(decode_entities("&nbsp;"), "\u{a0}");
    assert_eq!(decode_entities("Fish & Chips &bogus; &"), "Fish & Chips &bogus; &", "unknown entities stay");
}

#[test]
fn wowhead_html_becomes_lines_with_two_columns() {
    let html = r#"<table><tr><td><!--nstart--><b class="q5">Thunderfury, Blessed Blade</b><!--nend--><span class="q"><br>Item Level <!--ilvl-->80</span><!--bo--><br>Binds when picked up<br>Unique<table width="100%"><tr><td>One-Hand</td><th><!--scstart2:7--><span class="q1">Sword</span><!--scend--></th></tr></table><table width="100%"><tr>
    <td><span><!--dmg-->44 - 84 Damage</span></td>
    <th>Speed <!--spd-->1.90</th></tr></table><!--dps-->(33.7 damage per second)<br>+5 Agility<br><span class="q2">Chance on hit: Blasts &amp; jumps.</span></td></tr></table><table><tr><td>Requires Level 60</td></tr></table>"#;
    assert_eq!(
        html_to_lines(html),
        vec![
            line("Thunderfury, Blessed Blade", None),
            line("Item Level 80", None),
            line("Binds when picked up", None),
            line("Unique", None),
            line("One-Hand", Some("Sword")),
            line("44 - 84 Damage", Some("Speed 1.90")),
            line("(33.7 damage per second)", None),
            line("+5 Agility", None),
            line("Chance on hit: Blasts & jumps.", None),
            line("Requires Level 60", None),
        ]
    );
}

#[test]
fn empty_or_tag_only_html_gives_no_lines() {
    assert!(html_to_lines("").is_empty());
    assert!(html_to_lines("<table><tr><td><br><br/></td></tr></table><!-- x -->").is_empty());
    assert_eq!(html_to_lines("plain"), vec![line("plain", None)]);
    assert_eq!(html_to_lines("<script>var x = 1;</script>ok"), vec![line("ok", None)], "script text is dropped");
}

#[test]
fn answer_json_puts_the_name_first_in_its_quality() {
    let body = r#"{"name":"Linen Cloth","quality":1,"icon":"inv_fabric_linen_01","tooltip":"<b class=\"q1\">Linen Cloth</b><br>Crafting Reagent"}"#;
    let t = parse_answer(body).unwrap();
    assert_eq!(t.quality, Some(1));
    assert_eq!(t.lines, vec![line("Linen Cloth", None), line("Crafting Reagent", None)]);
    // HTML not starting with the name: the name is added on top.
    let t = parse_answer(r#"{"name":"X","quality":3,"tooltip":"Unique"}"#).unwrap();
    assert_eq!(t.lines, vec![line("X", None), line("Unique", None)]);
    assert!(parse_answer(r#"{"error":"not found"}"#).is_err(), "no name");
    assert!(parse_answer("<html>").is_err(), "not JSON");
}

// ---------------------------------------------------------------- fetch with a fake client

struct Fake {
    answer: Result<HttpResponse, String>,
    calls: Mutex<Vec<(String, Vec<(String, String)>, Duration)>>,
}

impl Fake {
    fn new(answer: Result<HttpResponse, String>) -> Self {
        Self { answer, calls: Mutex::new(Vec::new()) }
    }
}

impl HttpClient for Fake {
    fn get(&self, url: &str, headers: &[(String, String)], timeout: Duration) -> Result<HttpResponse, String> {
        self.calls.lock().unwrap().push((url.into(), headers.to_vec(), timeout));
        self.answer.clone()
    }
    fn post(&self, _: &str, _: &[(String, String)], _: &str, _: Duration) -> Result<HttpResponse, String> {
        panic!("never posts")
    }
}

#[test]
fn fetch_success_sends_a_polite_get_in_spanish() {
    let fake = Fake::new(Ok(HttpResponse { status: 200, body: r#"{"name":"Copper Ore","quality":1,"tooltip":"<b>Copper Ore</b>"}"#.into() }));
    let t = fetch(&fake, 2770, env::CLASSIC_ERA).unwrap();
    assert_eq!(t.name(), Some("Copper Ore"));
    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "https://nether.wowhead.com/tooltip/item/2770?dataEnv=4&locale=6");
    assert!(calls[0].1.iter().any(|(k, v)| k == "User-Agent" && v == &user_agent()));
    assert_eq!(calls[0].2, TIMEOUT);
    assert!(TIMEOUT <= Duration::from_secs(5));
}

#[test]
fn fetch_failures_are_errors() {
    let http_error = Fake::new(Ok(HttpResponse { status: 404, body: r#"{"error":"x"}"#.into() }));
    assert_eq!(fetch(&http_error, 1, 1).unwrap_err(), "HTTP 404");
    let bad_json = Fake::new(Ok(HttpResponse { status: 200, body: "<html>oops".into() }));
    assert!(fetch(&bad_json, 1, 1).unwrap_err().starts_with("bad JSON"));
    let timeout = Fake::new(Err("timeout: global".into()));
    assert_eq!(fetch(&timeout, 1, 1).unwrap_err(), "timeout: global");
}

#[test]
fn cache_allows_one_request_per_item_and_keeps_results() {
    let now = Instant::now();
    let mut cache = WowheadCache::default();
    assert!(cache.begin(5, now), "first hover starts a request");
    assert!(!cache.begin(5, now), "no duplicate while in flight");
    assert_eq!(cache.get(5), Some(&WowheadState::Loading));
    cache.finish(5, Ok(tip("Ore")), now);
    assert!(!cache.begin(5, now + Duration::from_secs(3600)), "a success is never asked again");
    assert_eq!(cache.get(5), Some(&WowheadState::Ready(tip("Ore"))));
}

#[test]
fn a_failure_is_cached_and_retried_only_after_a_few_minutes() {
    let now = Instant::now();
    let mut cache = WowheadCache::default();
    let fake = Fake::new(Err("timeout".into()));
    assert!(cache.begin(7, now));
    cache.finish(7, fetch(&fake, 7, 1), now);
    assert!(matches!(cache.get(7), Some(WowheadState::Failed { .. })));
    assert!(!cache.begin(7, now + Duration::from_secs(10)), "a failing id is not hammered");
    assert!(RETRY_AFTER <= Duration::from_secs(300), "retried after a few minutes at most");
    assert!(cache.begin(7, now + RETRY_AFTER), "retried after the delay");
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}

#[test]
fn requests_at_once_are_capped_and_the_cache_is_bounded() {
    let now = Instant::now();
    let mut cache = WowheadCache::default();
    for id in 0..MAX_IN_FLIGHT as u64 {
        assert!(cache.begin(id, now));
    }
    assert!(!cache.begin(999, now), "over the in-flight cap: try again on a later frame");
    cache.finish(0, Err("x".into()), now);
    assert!(cache.begin(999, now), "a finished request frees a slot");
    for id in 1000..(1000 + TOOLTIP_CACHE_CAP as u64 + 5) {
        cache.insert_ready(id, tip("x"));
    }
    assert_eq!(cache.len(), TOOLTIP_CACHE_CAP);
}

// ---------------------------------------------------------------- panel choice

#[test]
fn addon_mode_is_unchanged_by_wowhead_data() {
    let addon = tip("From game");
    let wh = WowheadState::Ready(tip("From web"));
    assert_eq!(panel_view(ItemTooltips::Off, Some(&addon), Some(&wh)), None, "off: no panel");
    assert_eq!(panel_view(ItemTooltips::Addon, Some(&addon), Some(&wh)), Some(PanelView::Tip { tip: &addon, note: None }));
    assert_eq!(panel_view(ItemTooltips::Addon, None, Some(&wh)), Some(PanelView::Message(LOADING_TEXT)));
    assert_eq!(LOADING_TEXT, "Cargando datos…");
}

#[test]
fn wowhead_mode_shows_web_data_loading_or_the_addon_fallback() {
    let addon = tip("From game");
    let web = tip("From web");
    let ready = WowheadState::Ready(web.clone());
    let failed = WowheadState::Failed { at: Instant::now(), error: "HTTP 500".into() };
    assert_eq!(panel_view(ItemTooltips::Wowhead, Some(&addon), Some(&ready)), Some(PanelView::Tip { tip: &web, note: Some("Wowhead") }));
    assert_eq!(panel_view(ItemTooltips::Wowhead, Some(&addon), None), Some(PanelView::Message("Cargando datos…")));
    assert_eq!(panel_view(ItemTooltips::Wowhead, None, Some(&WowheadState::Loading)), Some(PanelView::Message("Cargando datos…")));
    assert_eq!(
        panel_view(ItemTooltips::Wowhead, Some(&addon), Some(&failed)),
        Some(PanelView::Tip { tip: &addon, note: Some("WoW (addon)") }),
        "failure: the addon's data when the cache has it"
    );
    assert_eq!(panel_view(ItemTooltips::Wowhead, None, Some(&failed)), Some(PanelView::Message(NO_WOWHEAD_TEXT)));
    assert_eq!(NO_WOWHEAD_TEXT, "Sin datos de Wowhead");
}
