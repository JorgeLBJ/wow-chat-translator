//! Free, keyless Google Translate endpoint (unofficial, the one the Chrome dictionary
//! extension uses). The classic `translate_a/single?client=gtx` endpoint answered 429
//! from the user's network, so it is not used. Port of `google.js`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::http::{encode_component, HttpClient};
use crate::core::pipeline::{Translation, Translator};

const ENDPOINT: &str = "https://clients5.google.com/translate_a/t";
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct GoogleTranslator {
    http: Arc<dyn HttpClient>,
}

impl GoogleTranslator {
    pub fn new(http: Arc<dyn HttpClient>) -> Self {
        Self { http }
    }
}

impl Translator for GoogleTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        let url = format!(
            "{ENDPOINT}?client=dict-chrome-ex&sl=auto&tl={}&q={}",
            encode_component(target),
            encode_component(text)
        );
        let res = self.http.get(&url, &[], TIMEOUT)?;
        if !res.ok() {
            return Err(format!("Google Translate HTTP {}", res.status));
        }
        let data: Value = serde_json::from_str(&res.body).map_err(|_| "Google Translate: unexpected response".to_string())?;
        // Shape: [["translated", "detectedSource"]] (or ["translated"] when sl is fixed).
        let translation = |text: String, source: &str| Translation { text, source: source.into(), provider: None, fallback_reason: None };
        match data.get(0) {
            Some(Value::Array(first)) => {
                let text = match first.first() {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                let source = first.get(1).and_then(Value::as_str).unwrap_or("auto");
                Ok(translation(text, source))
            }
            Some(Value::String(s)) => Ok(translation(s.clone(), "auto")),
            _ => Err("Google Translate: unexpected response".into()),
        }
    }
}
