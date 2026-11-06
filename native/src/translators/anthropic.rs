//! Translator using the Anthropic Messages API over raw HTTP with a Console API key.
//! Port of `anthropic.js` (which used the official SDK; Rust has none).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::http::{encode_component, snippet, HttpClient, HttpResponse};
use super::prompt::{parse_llm_reply, system_prompt};
use super::ModelSource;
use crate::core::pipeline::{Translation, Translator};

pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-haiku-5-5";
/// API version header for raw HTTP (Claude API skill, `curl/examples.md` and
/// `shared/api-version-migration.md`). This version rejects `temperature`.
pub const ANTHROPIC_VERSION: &str = "2026-10-21";
pub const ANTHROPIC_API: &str = "https://api.anthropic.com";

const TIMEOUT: Duration = Duration::from_secs(20);
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// `output_config.effort` exists on the Claude 5 family; older models (e.g. Haiku 4.5)
/// answer 400 when it is sent.
pub fn supports_effort(model: &str) -> bool {
    let Some(rest) = model.strip_prefix("claude-") else { return false };
    ["fable-5", "mythos-5", "opus-5", "sonnet-5", "haiku-5"].iter().any(|p| rest.starts_with(p))
}

pub struct AnthropicTranslator {
    http: Arc<dyn HttpClient>,
    api_key: String,
    model: String,
}

impl AnthropicTranslator {
    pub fn new(http: Arc<dyn HttpClient>, api_key: &str, model: &str) -> Self {
        Self { http, api_key: api_key.to_string(), model: model.to_string() }
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-api-key".to_string(), self.api_key.clone()),
            ("anthropic-version".to_string(), ANTHROPIC_VERSION.to_string()),
        ]
    }

    /// Like the SDK with `maxRetries: 1`: one retry on connection errors, 408, 409,
    /// 429 and 5xx.
    fn with_retry(&self, send: impl Fn() -> Result<HttpResponse, String>) -> Result<HttpResponse, String> {
        let retryable = |r: &Result<HttpResponse, String>| match r {
            Err(_) => true,
            Ok(res) => matches!(res.status, 408 | 409 | 429) || res.status >= 500,
        };
        let first = send();
        if !retryable(&first) {
            return first;
        }
        std::thread::sleep(RETRY_DELAY);
        send()
    }
}

fn http_error(res: &HttpResponse) -> String {
    let message = serde_json::from_str::<Value>(&res.body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| snippet(&res.body, 200));
    if message.is_empty() {
        format!("Anthropic HTTP {}", res.status)
    } else {
        format!("Anthropic HTTP {}: {message}", res.status)
    }
}

impl Translator for AnthropicTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": 1024,
            "system": system_prompt(target),
            "messages": [{ "role": "user", "content": text }],
        });
        // Short chat lines need no deep reasoning; low effort keeps it fast and cheap.
        if supports_effort(&self.model) {
            body["output_config"] = json!({ "effort": "low" });
        }
        let body = body.to_string();
        let url = format!("{ANTHROPIC_API}/v1/messages");
        let res = self.with_retry(|| self.http.post(&url, &self.headers(), &body, TIMEOUT))?;
        if !res.ok() {
            return Err(http_error(&res));
        }
        let data: Value = serde_json::from_str(&res.body).map_err(|_| "Claude: unexpected response".to_string())?;
        if data["stop_reason"] == "refusal" {
            return Err("Claude declined to translate this message".into());
        }
        let reply: String = data["content"]
            .as_array()
            .map(|blocks| {
                blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("")
            })
            .unwrap_or_default();
        let parsed = parse_llm_reply(&reply);
        if parsed.text.is_empty() {
            return Err("Claude: empty reply".into());
        }
        Ok(parsed)
    }
}

impl ModelSource for AnthropicTranslator {
    /// All pages of `GET /v1/models` (`limit`, `after_id` / `has_more`, `last_id`).
    fn list_models(&self) -> Result<Vec<String>, String> {
        let mut ids = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut url = format!("{ANTHROPIC_API}/v1/models?limit=1000");
            if let Some(id) = &after {
                url.push_str(&format!("&after_id={}", encode_component(id)));
            }
            let res = self.with_retry(|| self.http.get(&url, &self.headers(), TIMEOUT))?;
            if !res.ok() {
                return Err(http_error(&res));
            }
            let data: Value = serde_json::from_str(&res.body).map_err(|_| "Anthropic: unexpected response".to_string())?;
            if let Some(models) = data["data"].as_array() {
                ids.extend(models.iter().filter_map(|m| m["id"].as_str()).map(String::from));
            }
            match (data["has_more"].as_bool(), data["last_id"].as_str()) {
                (Some(true), Some(last)) if after.as_deref() != Some(last) => after = Some(last.to_string()),
                _ => return Ok(ids),
            }
        }
    }
}
