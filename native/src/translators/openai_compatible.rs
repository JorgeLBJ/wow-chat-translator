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

impl Translator for OpenAICompatibleTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        let body = json!({
            "model": self.model,
            "temperature": 0.2,
            "max_tokens": 400,
            "messages": [
                { "role": "system", "content": system_prompt(target) },
                { "role": "user", "content": text },
            ],
        });
        let res = self.http.post(&format!("{}/chat/completions", self.root), &self.headers(), &body.to_string(), TIMEOUT)?;
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
