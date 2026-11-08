// Ported from app/test/core.test.js (google, openai-compatible, anthropic, claude code,
// parseLlmReply, cache, fallback, keyBinding) plus the native-only pieces: DPAPI key
// storage, provider+URL bound key reading, the translator factory and the Claude Code
// CLI argument list / result parsing.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use wow_chat_translator::core::pipeline::{Translation, Translator};
use wow_chat_translator::core::prefs::Config;
use wow_chat_translator::secret::{decrypt_api_key, encrypt_api_key, read_api_key};
use wow_chat_translator::translators::anthropic::{AnthropicTranslator, ANTHROPIC_DEFAULT_MODEL, ANTHROPIC_VERSION};
use wow_chat_translator::translators::cache::CachedTranslator;
use wow_chat_translator::translators::claude_code::{
    claude_args, parse_claude_result, ClaudeCodeTranslator, CliOutput, CliRunner, ProcessRunner, CLAUDE_CODE_MODELS,
};
use wow_chat_translator::translators::fallback::FallbackTranslator;
use wow_chat_translator::translators::google::GoogleTranslator;
use wow_chat_translator::translators::http::{HttpClient, HttpResponse};
use wow_chat_translator::translators::key_binding::key_binding;
use wow_chat_translator::translators::openai_compatible::OpenAICompatibleTranslator;
use wow_chat_translator::translators::prompt::{parse_llm_reply, system_prompt};
use wow_chat_translator::translators::{build_translator_with, effective_url, ModelSource};

// ---------------------------------------------------------------- fakes

#[derive(Clone, Debug)]
struct Call {
    method: &'static str,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout: Duration,
}

impl Call {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(self.body.as_deref().unwrap()).unwrap()
    }
}

/// Answers each request with the next queued response (the last one repeats).
#[derive(Default)]
struct FakeHttp {
    responses: Mutex<Vec<Result<HttpResponse, String>>>,
    calls: Mutex<Vec<Call>>,
}

impl FakeHttp {
    fn new(responses: Vec<Result<HttpResponse, String>>) -> Arc<Self> {
        Arc::new(Self { responses: Mutex::new(responses), calls: Mutex::new(Vec::new()) })
    }
    fn json(status: u16, body: Value) -> Arc<Self> {
        Self::new(vec![Ok(HttpResponse { status, body: body.to_string() })])
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn next(&self) -> Result<HttpResponse, String> {
        let mut r = self.responses.lock().unwrap();
        if r.len() > 1 {
            r.remove(0)
        } else {
            r[0].clone()
        }
    }
}

impl HttpClient for FakeHttp {
    fn get(&self, url: &str, headers: &[(String, String)], timeout: Duration) -> Result<HttpResponse, String> {
        self.calls.lock().unwrap().push(Call { method: "GET", url: url.into(), headers: headers.to_vec(), body: None, timeout });
        self.next()
    }
    fn post(&self, url: &str, headers: &[(String, String)], body: &str, timeout: Duration) -> Result<HttpResponse, String> {
        self.calls.lock().unwrap().push(Call {
            method: "POST",
            url: url.into(),
            headers: headers.to_vec(),
            body: Some(body.into()),
            timeout,
        });
        self.next()
    }
}

fn ok(text: &str, source: &str) -> Translation {
    Translation { text: text.into(), source: source.into(), provider: None, fallback_reason: None }
}

struct FnTranslator<F: Fn(&str, &str) -> Result<Translation, String> + Send + Sync>(F, AtomicUsize);

impl<F: Fn(&str, &str) -> Result<Translation, String> + Send + Sync> Translator for FnTranslator<F> {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        self.1.fetch_add(1, Ordering::SeqCst);
        (self.0)(text, target)
    }
}

fn counting<F: Fn(&str, &str) -> Result<Translation, String> + Send + Sync>(f: F) -> Arc<FnTranslator<F>> {
    Arc::new(FnTranslator(f, AtomicUsize::new(0)))
}

/// Shares one translator between the test and the wrapper under test.
struct Shared<T: Translator>(Arc<T>);
impl<T: Translator> Translator for Shared<T> {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        self.0.translate(text, target)
    }
}

// ---------------------------------------------------------------- google

#[test]
fn google_builds_the_request_and_returns_translation_with_detected_source() {
    let http = FakeHttp::json(200, serde_json::json!([["Hola. ¿Cómo estás?", "en"]]));
    let tr = GoogleTranslator::new(http.clone());
    assert_eq!(tr.translate("Hello. How are you?", "es").unwrap(), ok("Hola. ¿Cómo estás?", "en"));
    let calls = http.calls();
    assert_eq!(calls[0].method, "GET");
    assert_eq!(
        calls[0].url,
        "https://clients5.google.com/translate_a/t?client=dict-chrome-ex&sl=auto&tl=es&q=Hello.%20How%20are%20you%3F"
    );
}

#[test]
fn google_reads_a_plain_string_reply() {
    let http = FakeHttp::json(200, serde_json::json!(["hola"]));
    assert_eq!(GoogleTranslator::new(http).translate("hi", "es").unwrap(), ok("hola", "auto"));
}

#[test]
fn google_throws_on_http_errors_and_unexpected_replies() {
    let http = FakeHttp::json(429, serde_json::json!({}));
    let err = GoogleTranslator::new(http).translate("hi", "es").unwrap_err();
    assert!(err.contains("429"), "{err}");
    let http = FakeHttp::json(200, serde_json::json!({"x": 1}));
    assert!(GoogleTranslator::new(http).translate("hi", "es").is_err());
}

// ---------------------------------------------------------------- cache

#[test]
fn cache_calls_the_underlying_translator_once_per_text_and_target() {
    let inner = counting(|t, _| Ok(ok(&format!("es:{t}"), "en")));
    let tr = CachedTranslator::new(Box::new(Shared(inner.clone())), 10);
    tr.translate("ty", "es").unwrap();
    tr.translate("  TY ", "es").unwrap();
    assert_eq!(inner.1.load(Ordering::SeqCst), 1);
    tr.translate("ty", "en").unwrap();
    assert_eq!(inner.1.load(Ordering::SeqCst), 2);
}

#[test]
fn cache_does_not_cache_failures() {
    let fail = Arc::new(Mutex::new(true));
    let f = fail.clone();
    let inner = counting(move |_, _| if *f.lock().unwrap() { Err("x".into()) } else { Ok(ok("ok", "en")) });
    let tr = CachedTranslator::new(Box::new(Shared(inner)), 1000);
    assert!(tr.translate("a", "es").is_err());
    *fail.lock().unwrap() = false;
    assert_eq!(tr.translate("a", "es").unwrap(), ok("ok", "en"));
}

#[test]
fn cache_evicts_the_least_recently_used_entry() {
    let inner = counting(|t, _| Ok(ok(t, "en")));
    let tr = CachedTranslator::new(Box::new(Shared(inner.clone())), 2);
    tr.translate("a", "es").unwrap();
    tr.translate("b", "es").unwrap();
    tr.translate("a", "es").unwrap(); // a is now the most recent
    tr.translate("c", "es").unwrap(); // evicts b
    assert_eq!(inner.1.load(Ordering::SeqCst), 3);
    tr.translate("a", "es").unwrap();
    assert_eq!(inner.1.load(Ordering::SeqCst), 3);
    tr.translate("b", "es").unwrap();
    assert_eq!(inner.1.load(Ordering::SeqCst), 4);
}

// ---------------------------------------------------------------- parseLlmReply / prompt

#[test]
fn parse_llm_reply_reads_the_lang_and_text_lines() {
    assert_eq!(parse_llm_reply("LANG: en\nTEXT: Busco más para ZG, falta sanador"), ok("Busco más para ZG, falta sanador", "en"));
}

#[test]
fn parse_llm_reply_keeps_multi_line_translations_and_tolerates_spacing_and_case() {
    assert_eq!(parse_llm_reply("lang:PT\ntext:  línea uno\nlínea dos"), ok("línea uno\nlínea dos", "pt"));
}

#[test]
fn parse_llm_reply_falls_back_to_the_whole_reply_when_the_format_is_missing() {
    assert_eq!(parse_llm_reply("  hola a todos  "), ok("hola a todos", "auto"));
}

#[test]
fn system_prompt_names_the_language_and_mentions_ocr_and_placeholders() {
    let es = system_prompt("es");
    assert!(es.starts_with("You translate World of Warcraft chat messages into Spanish.\n"));
    assert!(es.contains("OCR"));
    assert!(es.contains("[0]"));
    assert!(es.ends_with("LANG: <ISO 639-1 code of the original message language>\nTEXT: <translation>"));
    assert!(system_prompt("en").contains("into English."));
    assert!(system_prompt("fr").contains("into fr."));
    assert_eq!(es.lines().count(), 17);
}

// ---------------------------------------------------------------- openai-compatible

fn chat_reply(content: &str) -> Arc<FakeHttp> {
    FakeHttp::json(200, serde_json::json!({"choices": [{"message": {"content": content}}]}))
}

#[test]
fn openai_posts_a_chat_completion_with_model_key_and_target_language() {
    let http = chat_reply("LANG: en\nTEXT: Busco más para ZG");
    let tr = OpenAICompatibleTranslator::new(http.clone(), "https://openrouter.ai/api/v1/", Some("k-123".into()), "m/x");
    assert_eq!(tr.translate("LFM ZG need heal", "es").unwrap(), ok("Busco más para ZG", "en"));
    let call = &http.calls()[0];
    assert_eq!(call.url, "https://openrouter.ai/api/v1/chat/completions");
    assert_eq!(call.method, "POST");
    assert_eq!(call.header("Authorization"), Some("Bearer k-123"));
    assert_eq!(call.header("Content-Type"), Some("application/json"));
    assert_eq!(call.header("X-Title"), Some("WoW Chat Translator"));
    assert_eq!(call.timeout, Duration::from_secs(20));
    let body = call.json();
    assert_eq!(body["model"], "m/x");
    assert_eq!(body["temperature"], 0.2);
    // Room for the answer even when the model reasons first.
    assert_eq!(body["max_tokens"], 1500);
    // OpenRouter: ask the model not to reason (a translation needs none).
    assert_eq!(body["reasoning"], serde_json::json!({"enabled": false}));
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("Spanish"));
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["messages"][1]["content"], "LFM ZG need heal");
}

#[test]
fn openai_omits_the_authorization_header_when_no_key_is_set() {
    let http = chat_reply("LANG: en\nTEXT: hola");
    OpenAICompatibleTranslator::new(http.clone(), "http://localhost:1234/v1", None, "local").translate("hi", "es").unwrap();
    assert_eq!(http.calls()[0].header("Authorization"), None);
    let http = chat_reply("LANG: en\nTEXT: hola");
    OpenAICompatibleTranslator::new(http.clone(), "http://localhost:1234/v1", Some(String::new()), "local")
        .translate("hi", "es")
        .unwrap();
    assert_eq!(http.calls()[0].header("Authorization"), None);
}

#[test]
fn openai_sends_no_reasoning_field_to_other_servers() {
    let http = chat_reply("LANG: en
TEXT: hola");
    OpenAICompatibleTranslator::new(http.clone(), "http://localhost:1234/v1", None, "local").translate("hi", "es").unwrap();
    assert!(http.calls()[0].json().get("reasoning").is_none());
}

#[test]
fn openai_retries_with_low_effort_when_openrouter_says_reasoning_is_mandatory() {
    let mandatory = r#"{"error":{"code":400,"message":"Reasoning is mandatory for this endpoint and cannot be disabled."}}"#;
    let http = FakeHttp::new(vec![
        Ok(HttpResponse { status: 400, body: mandatory.into() }),
        Ok(HttpResponse { status: 200, body: serde_json::json!({"choices": [{"message": {"content": "LANG: en
TEXT: hola"}}]}).to_string() }),
    ]);
    let tr = OpenAICompatibleTranslator::new(http.clone(), "https://openrouter.ai/api/v1", Some("k".into()), "m/x");
    assert_eq!(tr.translate("hi", "es").unwrap(), ok("hola", "en"));
    let calls = http.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].json()["reasoning"], serde_json::json!({"effort": "low"}));
    // Any other 400 is an error, without a retry.
    let http = FakeHttp::new(vec![Ok(HttpResponse { status: 400, body: "bad model".into() })]);
    let err = OpenAICompatibleTranslator::new(http.clone(), "https://openrouter.ai/api/v1", None, "m").translate("hi", "es").unwrap_err();
    assert_eq!(err, "LLM HTTP 400: bad model");
    assert_eq!(http.calls().len(), 1);
}

#[test]
fn openai_says_when_the_reply_was_cut_by_the_token_limit() {
    let http = FakeHttp::json(200, serde_json::json!({"choices": [{"message": {"content": ""}, "finish_reason": "length"}]}));
    let err = OpenAICompatibleTranslator::new(http, "http://x/v1", None, "m").translate("hi", "es").unwrap_err();
    assert_eq!(err, "LLM: la respuesta se cortó (el modelo agotó los tokens, quizá razonando)");
}

#[test]
fn openai_asks_for_english_when_the_target_is_en() {
    let http = chat_reply("LANG: es\nTEXT: anyone for Riptear?");
    OpenAICompatibleTranslator::new(http.clone(), "http://x/v1", None, "m").translate("alguien para Riptear?", "en").unwrap();
    assert!(http.calls()[0].json()["messages"][0]["content"].as_str().unwrap().contains("English"));
}

#[test]
fn openai_tells_the_model_to_keep_link_placeholders() {
    let http = chat_reply("LANG: en\nTEXT: vendo [0]");
    OpenAICompatibleTranslator::new(http.clone(), "http://x/v1", None, "m").translate("wts [0]", "es").unwrap();
    assert!(http.calls()[0].json()["messages"][0]["content"].as_str().unwrap().contains("[0]"));
}

#[test]
fn openai_throws_on_http_errors_and_empty_replies() {
    let http = FakeHttp::new(vec![Ok(HttpResponse { status: 401, body: "no key".into() })]);
    let err = OpenAICompatibleTranslator::new(http, "http://x/v1", None, "m").translate("hi", "es").unwrap_err();
    assert_eq!(err, "LLM HTTP 401: no key");
    let err = OpenAICompatibleTranslator::new(chat_reply(""), "http://x/v1", None, "m").translate("hi", "es").unwrap_err();
    assert_eq!(err, "LLM: empty reply");
    let http = FakeHttp::new(vec![Err("connection refused".into())]);
    assert!(OpenAICompatibleTranslator::new(http, "http://x/v1", None, "m").translate("hi", "es").is_err());
}

#[test]
fn openai_lists_model_ids_from_models() {
    let http = FakeHttp::json(200, serde_json::json!({"data": [{"id": "qwen2.5-7b"}, {"id": "llama-3.1-8b"}, {"x": 1}]}));
    let tr = OpenAICompatibleTranslator::new(http.clone(), "http://localhost:1234/v1", Some("k".into()), "");
    assert_eq!(tr.list_models().unwrap(), vec!["qwen2.5-7b", "llama-3.1-8b"]);
    let call = &http.calls()[0];
    assert_eq!(call.method, "GET");
    assert_eq!(call.url, "http://localhost:1234/v1/models");
    assert_eq!(call.header("Authorization"), Some("Bearer k"));
    let http = FakeHttp::json(500, serde_json::json!({}));
    assert!(OpenAICompatibleTranslator::new(http, "http://x/v1", None, "").list_models().unwrap_err().contains("500"));
}

// ---------------------------------------------------------------- anthropic

fn claude_ok(text: &str) -> Arc<FakeHttp> {
    FakeHttp::json(
        200,
        serde_json::json!({"stop_reason": "end_turn", "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": text}]}),
    )
}

#[test]
fn anthropic_sends_system_prompt_model_and_low_effort_and_parses_the_reply() {
    let http = claude_ok("LANG: en\nTEXT: Busco más para ZG");
    let tr = AnthropicTranslator::new(http.clone(), "sk-ant-test", "claude-haiku-5-5");
    assert_eq!(tr.translate("LFM ZG", "es").unwrap(), ok("Busco más para ZG", "en"));
    let call = &http.calls()[0];
    assert_eq!(call.method, "POST");
    assert_eq!(call.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(call.header("x-api-key"), Some("sk-ant-test"));
    assert_eq!(call.header("anthropic-version"), Some(ANTHROPIC_VERSION));
    assert_eq!(call.header("content-type"), Some("application/json"));
    assert_eq!(call.header("Authorization"), None);
    let req = call.json();
    assert_eq!(req["model"], "claude-haiku-5-5");
    assert_eq!(req["max_tokens"], 1024);
    assert!(req["system"].as_str().unwrap().contains("Spanish"));
    assert_eq!(req["messages"], serde_json::json!([{"role": "user", "content": "LFM ZG"}]));
    assert_eq!(req["output_config"], serde_json::json!({"effort": "low"}));
    assert!(req.get("temperature").is_none());
    assert_eq!(ANTHROPIC_VERSION, "2026-10-21");
    assert_eq!(ANTHROPIC_DEFAULT_MODEL, "claude-haiku-5-5");
}

#[test]
fn anthropic_sends_effort_only_to_claude_5_models() {
    for (model, effort) in [
        ("claude-haiku-4-5", false),
        ("claude-sonnet-4-6", false),
        ("claude-opus-5-5", true),
        ("claude-fable-5-1", true),
        ("claude-mythos-5-1", true),
        ("claude-sonnet-5", true),
        ("my-claude-opus-5", false),
    ] {
        let http = claude_ok("LANG: en\nTEXT: hola");
        AnthropicTranslator::new(http.clone(), "k", model).translate("hi", "es").unwrap();
        let req = http.calls()[0].json();
        assert_eq!(req.get("output_config").is_some(), effort, "{model}");
        assert!(req.get("temperature").is_none());
    }
}

#[test]
fn anthropic_throws_on_refusals_so_the_google_fallback_takes_over() {
    let http = FakeHttp::json(200, serde_json::json!({"stop_reason": "refusal", "content": []}));
    assert!(AnthropicTranslator::new(http, "k", "claude-haiku-5-5").translate("x", "es").is_err());
}

#[test]
fn anthropic_throws_on_http_errors_and_empty_replies_and_retries_once() {
    let http = FakeHttp::new(vec![Ok(HttpResponse { status: 401, body: "{\"error\":\"bad key\"}".into() })]);
    let err = AnthropicTranslator::new(http.clone(), "k", "m").translate("x", "es").unwrap_err();
    assert!(err.contains("401"), "{err}");
    assert_eq!(http.calls().len(), 1, "4xx is not retried");
    let err = AnthropicTranslator::new(claude_ok("  "), "k", "m").translate("x", "es").unwrap_err();
    assert_eq!(err, "Claude: empty reply");
    // One retry on overload, like the SDK's maxRetries: 1.
    let http = FakeHttp::new(vec![
        Ok(HttpResponse { status: 529, body: "overloaded".into() }),
        Ok(HttpResponse {
            status: 200,
            body: serde_json::json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "LANG: en\nTEXT: a"}, {"type": "text", "text": "b"}]}).to_string(),
        }),
    ]);
    assert_eq!(AnthropicTranslator::new(http.clone(), "k", "m").translate("x", "es").unwrap(), ok("ab", "en"));
    assert_eq!(http.calls().len(), 2);
}

#[test]
fn anthropic_lists_models_across_pages() {
    let http = FakeHttp::new(vec![
        Ok(HttpResponse {
            status: 200,
            body: serde_json::json!({"data": [{"id": "claude-haiku-5-5"}], "has_more": true, "first_id": "claude-haiku-5-5", "last_id": "claude-haiku-5-5"}).to_string(),
        }),
        Ok(HttpResponse {
            status: 200,
            body: serde_json::json!({"data": [{"id": "claude-sonnet-5-5"}], "has_more": false, "first_id": "claude-sonnet-5-5", "last_id": "claude-sonnet-5-5"}).to_string(),
        }),
    ]);
    let tr = AnthropicTranslator::new(http.clone(), "k", "m");
    assert_eq!(tr.list_models().unwrap(), vec!["claude-haiku-5-5", "claude-sonnet-5-5"]);
    let calls = http.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].method, "GET");
    assert_eq!(calls[0].url, "https://api.anthropic.com/v1/models?limit=1000");
    assert_eq!(calls[1].url, "https://api.anthropic.com/v1/models?limit=1000&after_id=claude-haiku-5-5");
    assert_eq!(calls[1].header("x-api-key"), Some("k"));
    assert_eq!(calls[1].header("anthropic-version"), Some(ANTHROPIC_VERSION));
}

// ---------------------------------------------------------------- fallback

#[test]
fn fallback_uses_the_primary_and_tags_the_result_with_its_name() {
    let google = counting(|_, _| Ok(ok("g", "en")));
    let tr = FallbackTranslator::new(
        "llm",
        Box::new(Shared(counting(|_, _| Ok(ok("a", "en"))))),
        "google",
        Box::new(Shared(google.clone())),
    );
    let res = tr.translate("x", "es").unwrap();
    assert_eq!(res, Translation { text: "a".into(), source: "en".into(), provider: Some("llm".into()), fallback_reason: None });
    assert_eq!(google.1.load(Ordering::SeqCst), 0);
}

#[test]
fn fallback_falls_back_to_the_secondary_when_the_primary_fails() {
    let tr = FallbackTranslator::new(
        "llm",
        Box::new(Shared(counting(|_, _| Err("down".into())))),
        "google",
        Box::new(Shared(counting(|_, _| Ok(ok("b", "en"))))),
    );
    let res = tr.translate("x", "es").unwrap();
    assert_eq!(
        res,
        Translation { text: "b".into(), source: "en".into(), provider: Some("google".into()), fallback_reason: Some("down".into()) }
    );
}

#[test]
fn fallback_fails_when_both_fail() {
    let tr = FallbackTranslator::new(
        "llm",
        Box::new(Shared(counting(|_, _| Err("down".into())))),
        "google",
        Box::new(Shared(counting(|_, _| Err("also down".into())))),
    );
    assert_eq!(tr.translate("x", "es").unwrap_err(), "also down");
}

// ---------------------------------------------------------------- key binding

#[test]
fn key_binding_is_the_same_for_equivalent_urls_of_the_same_provider() {
    assert_eq!(key_binding("openrouter", "https://OpenRouter.ai/api/v1/"), key_binding("openrouter", "https://openrouter.ai/api/v1"));
    assert_eq!(key_binding("openrouter", " https://openrouter.ai/api/v1// "), "openrouter|https://openrouter.ai/api/v1");
    assert_eq!(key_binding("anthropic", "https://api.anthropic.com"), "anthropic|https://api.anthropic.com");
    assert_eq!(key_binding("custom", "https://x.example:443/v1?a=1"), "custom|https://x.example/v1");
    assert_eq!(key_binding("lmstudio", "http://LocalHost:1234/v1"), "lmstudio|http://localhost:1234/v1");
    assert_eq!(key_binding("custom", "not a url"), "custom|not a url");
}

#[test]
fn key_binding_differs_when_the_provider_or_the_server_changes() {
    let saved = key_binding("openrouter", "https://openrouter.ai/api/v1");
    assert_ne!(key_binding("custom", "https://openrouter.ai/api/v1"), saved);
    assert_ne!(key_binding("openrouter", "https://evil.example/api/v1"), saved);
    assert_ne!(key_binding("lmstudio", "http://localhost:1234/v1"), saved);
    assert_ne!(key_binding("openrouter", "http://openrouter.ai/api/v1"), saved);
    assert_ne!(key_binding("openrouter", "https://openrouter.ai:8443/api/v1"), saved);
}

#[test]
fn effective_url_uses_the_provider_default_and_always_the_anthropic_endpoint() {
    assert_eq!(effective_url("openrouter", ""), "https://openrouter.ai/api/v1");
    assert_eq!(effective_url("lmstudio", ""), "http://localhost:1234/v1");
    assert_eq!(effective_url("custom", "http://my/v1"), "http://my/v1");
    assert_eq!(effective_url("custom", ""), "");
    assert_eq!(effective_url("anthropic", "https://evil.example"), "https://api.anthropic.com");
}

// ---------------------------------------------------------------- DPAPI + bound key

#[test]
fn dpapi_round_trips_and_the_ciphertext_does_not_contain_the_plaintext() {
    let plain = "sk-or-v1-plaintext-secret-0123456789";
    let enc = encrypt_api_key(plain).unwrap();
    assert!(!enc.contains(plain));
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD.decode(&enc).unwrap();
    assert!(!raw.windows(plain.len()).any(|w| w == plain.as_bytes()));
    let utf16: Vec<u8> = plain.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    assert!(!raw.windows(utf16.len()).any(|w| w == utf16.as_slice()));
    assert_eq!(decrypt_api_key(&enc).unwrap(), plain);
    assert!(decrypt_api_key("bm90IGRwYXBp").is_err()); // "not dpapi"
    assert!(decrypt_api_key("%%%").is_err());
}

fn config_with_key(provider: &str, url: &str, key: &str) -> Config {
    let mut config = Config::default();
    config.translator.provider = provider.into();
    config.translator.base_url = url.into();
    config.translator.model = "m".into();
    config.translator.api_key_enc = Some(encrypt_api_key(key).unwrap());
    config.translator.key_binding = Some(key_binding(provider, &effective_url(provider, url)));
    config
}

#[test]
fn read_api_key_returns_the_key_only_for_the_provider_and_url_it_was_saved_for() {
    let config = config_with_key("openrouter", "", "sk-or-1");
    assert_eq!(read_api_key(&config, "openrouter", "").as_deref(), Some("sk-or-1"));
    assert_eq!(read_api_key(&config, "openrouter", "https://OpenRouter.ai/api/v1/").as_deref(), Some("sk-or-1"));
    // Switching provider or server never hands the key to it.
    assert_eq!(read_api_key(&config, "custom", "https://openrouter.ai/api/v1"), None);
    assert_eq!(read_api_key(&config, "openrouter", "https://evil.example/api/v1"), None);
    assert_eq!(read_api_key(&config, "lmstudio", ""), None);
    assert_eq!(read_api_key(&config, "anthropic", ""), None);
}

#[test]
fn read_api_key_always_binds_anthropic_to_its_own_endpoint() {
    let config = config_with_key("anthropic", "https://whatever.example", "sk-ant-1");
    assert_eq!(read_api_key(&config, "anthropic", "https://evil.example").as_deref(), Some("sk-ant-1"));
    assert_eq!(read_api_key(&config, "custom", "https://api.anthropic.com"), None);
}

#[test]
fn read_api_key_treats_missing_or_undecryptable_keys_as_no_key() {
    let mut config = config_with_key("openrouter", "", "sk-or-1");
    config.translator.api_key_enc = Some("djEw".into()); // Electron safeStorage "v10" prefix, not DPAPI
    assert_eq!(read_api_key(&config, "openrouter", ""), None);
    config.translator.api_key_enc = None;
    assert_eq!(read_api_key(&config, "openrouter", ""), None);
    let mut config = config_with_key("openrouter", "", "sk-or-1");
    config.translator.key_binding = None;
    assert_eq!(read_api_key(&config, "openrouter", ""), None);
}

// ---------------------------------------------------------------- factory

fn settings(provider: &str, base_url: &str, model: &str) -> Config {
    let mut config = Config::default();
    config.translator.provider = provider.into();
    config.translator.base_url = base_url.into();
    config.translator.model = model.into();
    config
}

fn google_reply() -> Result<HttpResponse, String> {
    Ok(HttpResponse { status: 200, body: serde_json::json!([["hola", "en"]]).to_string() })
}

#[test]
fn factory_uses_google_only_for_google_or_without_a_model() {
    for config in [settings("google", "", "x"), settings("openrouter", "", "")] {
        let http = FakeHttp::new(vec![google_reply()]);
        let built = build_translator_with(&config, http.clone(), None, &|| None);
        assert_eq!(built.issue, None);
        let res = built.translator.translate("hi", "es").unwrap();
        assert_eq!(res.provider.as_deref(), Some("Google"));
        assert_eq!(res.fallback_reason, None);
        built.translator.translate("hi", "es").unwrap();
        assert_eq!(http.calls().len(), 1, "cached");
        assert!(http.calls()[0].url.starts_with("https://clients5.google.com/"));
    }
}

#[test]
fn factory_without_an_anthropic_key_uses_google_and_says_why() {
    let http = FakeHttp::new(vec![google_reply()]);
    let built = build_translator_with(&settings("anthropic", "", "claude-haiku-5-5"), http.clone(), None, &|| None);
    assert_eq!(built.issue.as_deref(), Some("Anthropic: Falta la clave API de Anthropic. Se usa Google."));
    assert_eq!(built.translator.translate("hi", "es").unwrap().provider.as_deref(), Some("Google"));
}

#[test]
fn factory_without_claude_code_installed_uses_google_and_says_why() {
    let http = FakeHttp::new(vec![google_reply()]);
    let built = build_translator_with(&settings("claudecode", "", "claude-haiku-5-5"), http, Some("sk-x".into()), &|| None);
    assert_eq!(
        built.issue.as_deref(),
        Some("Claude Code: Claude Code no está instalado en este PC (instálalo y ejecuta `claude auth login`). Se usa Google.")
    );
}

#[test]
fn factory_without_a_server_url_uses_google_and_says_why() {
    let http = FakeHttp::new(vec![google_reply()]);
    let built = build_translator_with(&settings("custom", "", "m"), http, None, &|| None);
    assert_eq!(built.issue.as_deref(), Some("Personalizado: Falta la URL del servidor. Se usa Google."));
}

#[test]
fn factory_builds_a_cached_llm_with_google_fallback() {
    let http = FakeHttp::new(vec![Err("connection refused".into()), google_reply()]);
    let built = build_translator_with(&settings("lmstudio", "", "qwen"), http.clone(), None, &|| None);
    assert_eq!(built.issue, None);
    let res = built.translator.translate("hi", "es").unwrap();
    assert_eq!(res.provider.as_deref(), Some("Google"));
    assert_eq!(res.fallback_reason.as_deref(), Some("connection refused"));
    let calls = http.calls();
    assert_eq!(calls[0].url, "http://localhost:1234/v1/chat/completions");
    assert_eq!(calls[0].header("Authorization"), None);
    built.translator.translate("hi", "es").unwrap();
    assert_eq!(http.calls().len(), 2, "cached");

    let http = chat_reply("LANG: en\nTEXT: hola");
    let built = build_translator_with(&settings("openrouter", "", "m/x"), http.clone(), Some("k-1".into()), &|| None);
    let res = built.translator.translate("hi", "es").unwrap();
    assert_eq!(res.provider.as_deref(), Some("OpenRouter"));
    assert_eq!(http.calls()[0].header("Authorization"), Some("Bearer k-1"));

    let http = claude_ok("LANG: en\nTEXT: hola");
    let built = build_translator_with(&settings("anthropic", "https://evil.example", "claude-haiku-5-5"), http.clone(), Some("k-2".into()), &|| None);
    assert_eq!(built.translator.translate("hi", "es").unwrap().provider.as_deref(), Some("Anthropic"));
    assert_eq!(http.calls()[0].url, "https://api.anthropic.com/v1/messages");
}

#[test]
fn factory_never_passes_the_stored_key_to_claude_code() {
    let http = FakeHttp::new(vec![google_reply()]);
    let exe = PathBuf::from("C:/does/not/exist/claude.exe");
    let found = exe.clone();
    let built = build_translator_with(&settings("claudecode", "", "claude-haiku-5-5"), http, Some("sk-secret".into()), &move || {
        Some(found.clone())
    });
    assert_eq!(built.issue, None);
    // The missing executable fails, so Google answers and says why; the key is in no argument.
    let res = built.translator.translate("hi", "es").unwrap();
    assert_eq!(res.provider.as_deref(), Some("Google"));
    assert!(res.fallback_reason.unwrap().starts_with("Claude Code: "));
    assert!(!claude_args("claude-haiku-5-5", &system_prompt("es")).iter().any(|a| a.contains("sk-secret")));
}

// ---------------------------------------------------------------- claude code

#[test]
fn claude_args_run_a_tool_less_settings_less_print_mode_query_with_the_chosen_model() {
    let args = claude_args("claude-haiku-5-5", "SYS");
    let expected: Vec<String> = [
        "-p",
        "--model",
        "claude-haiku-5-5",
        "--system-prompt",
        "SYS",
        "--tools",
        "",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--output-format",
        "json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(args, expected);
    assert_eq!(CLAUDE_CODE_MODELS, ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"]);
}

#[test]
fn parse_claude_result_reads_the_result_message() {
    let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"LANG: en\nTEXT: hola","session_id":"x"}"#;
    assert_eq!(parse_claude_result(out).unwrap(), ok("hola", "en"));
    // --verbose style: an array of messages, the result last.
    let out = r#"[{"type":"system"},{"type":"result","subtype":"success","is_error":false,"result":"LANG: en\nTEXT: hola"}]"#;
    assert_eq!(parse_claude_result(out).unwrap(), ok("hola", "en"));
}

#[test]
fn parse_claude_result_throws_when_claude_code_reports_an_error() {
    let out = r#"{"type":"result","subtype":"success","is_error":true,"result":"rate limited"}"#;
    assert_eq!(parse_claude_result(out).unwrap_err(), "Claude Code: rate limited");
    let out = r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#;
    assert_eq!(parse_claude_result(out).unwrap_err(), "Claude Code: error_max_turns");
    assert_eq!(parse_claude_result("not json").unwrap_err(), "Claude Code: no result");
    let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"   "}"#;
    assert_eq!(parse_claude_result(out).unwrap_err(), "Claude Code: empty reply");
}

struct FakeCli {
    running: AtomicUsize,
    peak: AtomicUsize,
    delay: Duration,
    stdout: String,
    calls: Mutex<Vec<(PathBuf, Vec<String>, String, Duration)>>,
}

impl FakeCli {
    fn new(delay_ms: u64, stdout: &str) -> Arc<Self> {
        Arc::new(Self {
            running: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay: Duration::from_millis(delay_ms),
            stdout: stdout.into(),
            calls: Mutex::new(Vec::new()),
        })
    }
}

impl CliRunner for FakeCli {
    fn run(&self, exe: &Path, args: &[String], stdin: &str, timeout: Duration) -> Result<CliOutput, String> {
        self.calls.lock().unwrap().push((exe.to_path_buf(), args.to_vec(), stdin.into(), timeout));
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        self.running.fetch_sub(1, Ordering::SeqCst);
        Ok(CliOutput { success: true, stdout: self.stdout.clone(), stderr: String::new() })
    }
}

const RESULT_OK: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"LANG: en\nTEXT: hola"}"#;

#[test]
fn claude_code_runs_the_installed_cli_with_the_text_on_stdin() {
    let cli = FakeCli::new(0, RESULT_OK);
    let exe = PathBuf::from("C:/Users/x/.local/bin/claude.exe");
    let tr = ClaudeCodeTranslator::new(exe.clone(), "claude-haiku-5-5", cli.clone());
    assert_eq!(tr.translate("-hi", "es").unwrap(), ok("hola", "en"));
    let calls = cli.calls.lock().unwrap();
    let (called_exe, args, stdin, timeout) = &calls[0];
    assert_eq!(called_exe, &exe);
    assert_eq!(args, &claude_args("claude-haiku-5-5", &system_prompt("es")));
    assert_eq!(stdin, "-hi");
    assert_eq!(*timeout, Duration::from_secs(30));
    assert_eq!(tr.list_models().unwrap(), CLAUDE_CODE_MODELS.to_vec());
}

#[test]
fn claude_code_runs_at_most_two_queries_at_once() {
    let cli = FakeCli::new(40, RESULT_OK);
    let tr = Arc::new(ClaudeCodeTranslator::new(PathBuf::from("claude.exe"), "m", cli.clone()));
    let handles: Vec<_> = (0..5)
        .map(|_| {
            let tr = tr.clone();
            std::thread::spawn(move || tr.translate("hi", "es").unwrap())
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(cli.peak.load(Ordering::SeqCst), 2);
    assert_eq!(cli.calls.lock().unwrap().len(), 5);
}

struct TimeoutCli;
impl CliRunner for TimeoutCli {
    fn run(&self, _: &Path, _: &[String], _: &str, timeout: Duration) -> Result<CliOutput, String> {
        Err(format!("sin respuesta tras {} s (time out)", timeout.as_secs()))
    }
}

#[test]
fn claude_code_reports_a_timeout_so_the_fallback_can_run() {
    let tr = ClaudeCodeTranslator::new(PathBuf::from("claude.exe"), "m", Arc::new(TimeoutCli));
    assert_eq!(tr.translate("hi", "es").unwrap_err(), "Claude Code: sin respuesta tras 30 s (time out)");
}

#[cfg(windows)]
#[test]
fn process_runner_kills_a_child_that_does_not_answer_in_time() {
    let ping = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("System32\\PING.EXE");
    let start = Instant::now();
    let err = ProcessRunner
        .run(&ping, &["-n".into(), "30".into(), "127.0.0.1".into()], "", Duration::from_millis(400))
        .unwrap_err();
    assert!(err.contains("time out"), "{err}");
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
}

#[cfg(windows)]
#[test]
fn process_runner_returns_stdout_and_feeds_stdin() {
    let cmd = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("System32\\findstr.exe");
    let out = ProcessRunner.run(&cmd, &["x".into()], "axb\nccc\n", Duration::from_secs(10)).unwrap();
    assert!(out.success);
    assert_eq!(out.stdout.trim(), "axb");
    let err = ProcessRunner.run(Path::new("C:/does/not/exist.exe"), &[], "", Duration::from_secs(1)).unwrap_err();
    assert!(!err.is_empty());
}

// ---------------------------------------------------------------- live (manual)
// cargo test --test translators -- --ignored --nocapture

#[test]
#[ignore = "live network call"]
fn live_google() {
    let tr = GoogleTranslator::new(Arc::new(wow_chat_translator::translators::http::UreqClient::new()));
    let start = Instant::now();
    let res = tr.translate("LFM ZG need heal, pst", "es").unwrap();
    println!("google: {res:?} in {:?}", start.elapsed());
}

#[test]
#[ignore = "live Claude Code call (uses the logged-in subscription)"]
fn live_claude_code() {
    let exe = wow_chat_translator::translators::claude_code::find_claude_executable().expect("claude installed");
    let tr = ClaudeCodeTranslator::new(exe, "claude-haiku-5-5", Arc::new(ProcessRunner));
    let start = Instant::now();
    let res = tr.translate("LFM ZG need heal, pst", "es");
    println!("claude code: {res:?} in {:?}", start.elapsed());
    res.unwrap();
}

// ---------------------------------------------------------------- E5: redirects and process trees

mod e5 {
    use super::*;
    use std::io::{BufRead, BufReader, Read as _, Write as _};
    use std::net::TcpListener;
    use wow_chat_translator::translators::claude_code::kill_tree_args;
    use wow_chat_translator::translators::http::{max_redirects_for, UreqClient, DEFAULT_MAX_REDIRECTS};

    /// A local server answering every request with `response`; counts requests and
    /// records whether any carried an `x-api-key` header.
    struct Server {
        port: u16,
        hits: Arc<AtomicUsize>,
        saw_key: Arc<AtomicUsize>,
    }

    fn serve(response: impl Fn(u16) -> String + Send + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let saw_key = Arc::new(AtomicUsize::new(0));
        let (h, k) = (hits.clone(), saw_key.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                h.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("x-api-key:") {
                        k.fetch_add(1, Ordering::SeqCst);
                    }
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);
                let _ = stream.write_all(response(port).as_bytes());
            }
        });
        Server { port, hits, saw_key }
    }

    fn redirect_to(target: u16) -> Server {
        serve(move |_| {
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{target}/v1/messages\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
        })
    }

    fn ok_server() -> Server {
        serve(|_| "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string())
    }

    fn h(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn requests_carrying_x_api_key_never_follow_redirects() {
        assert_eq!(max_redirects_for(&[h("x-api-key", "sk-ant")]), 0);
        assert_eq!(max_redirects_for(&[h("X-Api-Key", "sk-ant")]), 0);
        // Everything else keeps ureq's default (ureq never forwards Authorization).
        assert_eq!(max_redirects_for(&[h("Authorization", "Bearer k")]), DEFAULT_MAX_REDIRECTS);
        assert_eq!(max_redirects_for(&[]), DEFAULT_MAX_REDIRECTS);
    }

    #[test]
    fn anthropic_requests_are_configured_without_redirects() {
        let http = FakeHttp::json(200, serde_json::json!({ "content": [{ "type": "text", "text": "hola" }] }));
        let tr = AnthropicTranslator::new(http.clone(), "sk-ant-1", ANTHROPIC_DEFAULT_MODEL);
        let _ = tr.translate("hi", "es");
        let _ = tr.list_models();
        let calls = http.calls();
        assert!(calls.len() >= 2);
        for call in calls {
            assert_eq!(max_redirects_for(&call.headers), 0, "{}", call.url);
        }
    }

    #[test]
    fn ureq_client_does_not_send_x_api_key_to_a_redirect_target() {
        let other = ok_server();
        let first = redirect_to(other.port);
        let url = format!("http://127.0.0.1:{}/v1/messages", first.port);
        let res = UreqClient::new()
            .post(&url, &[h("x-api-key", "sk-ant-secret"), h("content-type", "application/json")], "{}", Duration::from_secs(5))
            .unwrap();
        assert_eq!(res.status, 307);
        assert_eq!(first.hits.load(Ordering::SeqCst), 1);
        assert_eq!(first.saw_key.load(Ordering::SeqCst), 1);
        assert_eq!(other.hits.load(Ordering::SeqCst), 0, "the redirect was followed");
        assert_eq!(other.saw_key.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ureq_client_still_follows_redirects_without_x_api_key() {
        let other = ok_server();
        let first = redirect_to(other.port);
        let url = format!("http://127.0.0.1:{}/v1/messages", first.port);
        let res = UreqClient::new().get(&url, &[h("Authorization", "Bearer k")], Duration::from_secs(5)).unwrap();
        assert_eq!(res, HttpResponse { status: 200, body: "ok".into() });
        assert_eq!(other.hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn kill_tree_args_target_the_whole_tree_by_pid() {
        assert_eq!(kill_tree_args(4321), vec!["/PID", "4321", "/T", "/F"]);
    }

    /// A `.cmd` shim (like npm's `claude.cmd`) whose grandchild keeps appending to a
    /// file: after the timeout kill the file must stop growing. The grandchild stops by
    /// itself after ~12 s, so a failing run leaves nothing behind for long.
    #[cfg(windows)]
    #[test]
    fn a_timed_out_cmd_shim_has_its_whole_process_tree_killed() {
        let dir = std::env::temp_dir().join(format!("wct-e5-kill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ticks = dir.join("ticks.txt");
        let shim = dir.join("shim.cmd");
        std::fs::write(
            &shim,
            "@echo off\r\npowershell -NoProfile -Command \"for($i=0;$i -lt 60;$i++){Add-Content -LiteralPath '%~dp0ticks.txt' -Value x; Start-Sleep -Milliseconds 200}\"\r\n",
        )
        .unwrap();
        let res = ProcessRunner.run(&shim, &[], "", Duration::from_secs(3));
        assert!(res.is_err(), "{res:?}");
        std::thread::sleep(Duration::from_millis(1000));
        let before = std::fs::metadata(&ticks).map(|m| m.len()).unwrap_or(0);
        std::thread::sleep(Duration::from_millis(2000));
        let after = std::fs::metadata(&ticks).map(|m| m.len()).unwrap_or(0);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(before > 0, "the grandchild never started");
        assert_eq!(before, after, "the grandchild kept running after the timeout");
    }
}
