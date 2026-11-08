//! Translator for any OpenAI-compatible chat completions API: OpenRouter
//! (https://openrouter.ai/api/v1), LM Studio (http://localhost:1234/v1), etc.
//! Port of `openai-compatible.js`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::http::{snippet, HttpClient};
use super::prompt::{parse_llm_reply, system_prompt};
use super::ModelSource;
use crate::core::pipeline::{Translation, Translator};

const TIMEOUT: Duration = Duration::from_secs(20);

pub struct OpenAICompatibleTranslator {
    http: Arc<dyn HttpClient>,
    root: String,
    api_key: Option<String>,
    model: String,
}

impl OpenAICompatibleTranslator {
    pub fn new(http: Arc<dyn HttpClient>, base_url: &str, api_key: Option<String>, model: &str) -> Self {
        Self {
            http,
            root: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.filter(|k| !k.is_empty()),
            model: model.to_string(),
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            // Optional attribution header used by OpenRouter; harmless elsewhere.
            ("X-Title".to_string(), "WoW Chat Translator".to_string()),
        ];
        if let Some(key) = &self.api_key {
            h.push(("Authorization".to_string(), format!("Bearer {key}")));
        }
        h
    }
}

/// Output budget. Reasoning models (e.g. DeepSeek on OpenRouter) spend tokens thinking
/// before answering; with 400 the answer itself was sometimes cut to nothing.
const MAX_TOKENS: u32 = 1500;

impl OpenAICompatibleTranslator {
    fn is_openrouter(&self) -> bool {
        self.root.contains("openrouter.ai")
    }

    fn body(&self, text: &str, target: &str, reasoning: Option<Value>) -> Value {
        let mut body = json!({
            "model": self.model,
            "temperature": 0.2,
            "max_tokens": MAX_TOKENS,
            "messages": [
                { "role": "system", "content": system_prompt(target) },
                { "role": "user", "content": text },
            ],
        });
        if let Some(r) = reasoning {
            body["reasoning"] = r;
        }
        body
    }
}

impl Translator for OpenAICompatibleTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        let url = format!("{}/chat/completions", self.root);
        // OpenRouter: a translation needs no reasoning, so ask for none (faster, and the
        // answer is never starved of tokens). Models where reasoning is mandatory answer
        // 400 to that; retry once with the lowest effort. Other servers get no extra field.
        let first = self.is_openrouter().then(|| json!({ "enabled": false }));
        let mut res = self.http.post(&url, &self.headers(), &self.body(text, target, first).to_string(), TIMEOUT)?;
        if self.is_openrouter() && res.status == 400 && res.body.to_lowercase().contains("mandatory") {
            let low = Some(json!({ "effort": "low" }));
            res = self.http.post(&url, &self.headers(), &self.body(text, target, low).to_string(), TIMEOUT)?;
        }
        if !res.ok() {
            let detail = snippet(&res.body, 200);
            return Err(if detail.is_empty() {
                format!("LLM HTTP {}", res.status)
            } else {
                format!("LLM HTTP {}: {detail}", res.status)
            });
        }
        let data: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
        let content = match &data["choices"][0]["message"]["content"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let parsed = parse_llm_reply(&content);
        if parsed.text.is_empty() {
            if data["choices"][0]["finish_reason"] == "length" {
                return Err("LLM: la respuesta se cortó (el modelo agotó los tokens, quizá razonando)".into());
            }
            return Err("LLM: empty reply".into());
        }
        Ok(parsed)
    }
}

impl ModelSource for OpenAICompatibleTranslator {
    fn list_models(&self) -> Result<Vec<String>, String> {
        let res = self.http.get(&format!("{}/models", self.root), &self.headers(), TIMEOUT)?;
        if !res.ok() {
            return Err(format!("LLM HTTP {}", res.status));
        }
        let data: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
        Ok(data["data"]
            .as_array()
            .map(|models| {
                models.iter().filter_map(|m| m["id"].as_str()).filter(|id| !id.is_empty()).map(String::from).collect()
            })
            .unwrap_or_default())
    }
}
