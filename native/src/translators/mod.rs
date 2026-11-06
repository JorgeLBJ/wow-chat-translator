//! Translators (Google, OpenAI-compatible, Anthropic API, Claude Code), the cache and
//! Google fallback around them, and the factory that builds the configured one.
//! Ported from `app/src/core/translators` and `buildTranslator` in `app/src/main.js`.

pub mod anthropic;
pub mod cache;
pub mod claude_code;
pub mod fallback;
pub mod google;
pub mod http;
pub mod key_binding;
pub mod openai_compatible;
pub mod prompt;

use std::path::PathBuf;
use std::sync::Arc;

use crate::core::pipeline::Translator;
use crate::core::prefs::{Config, TranslatorSettings};

use anthropic::{AnthropicTranslator, ANTHROPIC_API, ANTHROPIC_DEFAULT_MODEL};
use cache::{CachedTranslator, CACHE_MAX};
use claude_code::{ClaudeCodeTranslator, ProcessRunner, CLAUDE_CODE_MODELS, NOT_INSTALLED};
use fallback::{FallbackTranslator, Named};
use google::GoogleTranslator;
use http::{HttpClient, UreqClient};
use openai_compatible::OpenAICompatibleTranslator;

/// Translators that can also list the models their server offers ("Cargar modelos").
pub trait ModelSource {
    fn list_models(&self) -> Result<Vec<String>, String>;
}

/// An LLM translator: translates and lists models.
pub trait Llm: Translator + ModelSource + Send + Sync {}
impl<T: Translator + ModelSource + Send + Sync> Llm for T {}

/// Known providers: id, label shown to the user, default server URL.
pub const PROVIDERS: [(&str, &str, &str); 6] = [
    ("google", "Google", ""),
    ("openrouter", "OpenRouter", "https://openrouter.ai/api/v1"),
    ("lmstudio", "LM Studio", "http://localhost:1234/v1"),
    ("anthropic", "Anthropic", ANTHROPIC_API),
    ("claudecode", "Claude Code", ""),
    ("custom", "Personalizado", ""),
];

fn provider(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    PROVIDERS.iter().copied().find(|(p, _, _)| *p == id)
}

/// Label of a provider id, "LLM" for unknown ones.
pub fn provider_label(id: &str) -> &'static str {
    provider(id).map_or("LLM", |(_, label, _)| label)
}

/// The server a provider really talks to: Anthropic always uses its own endpoint,
/// whatever URL the form holds; others use the form's URL or their default.
pub fn effective_url(provider_id: &str, base_url: &str) -> String {
    if provider_id == "anthropic" {
        return ANTHROPIC_API.to_string();
    }
    if !base_url.is_empty() {
        return base_url.to_string();
    }
    provider(provider_id).map_or("", |(_, _, url)| url).to_string()
}

/// The LLM for some settings (without cache or fallback), for translating and for
/// "Probar" / "Cargar modelos". `api_key` is never given to Claude Code.
pub fn llm_for(
    settings: &TranslatorSettings,
    api_key: Option<String>,
    http: Arc<dyn HttpClient>,
    find_claude: &dyn Fn() -> Option<PathBuf>,
) -> Result<Box<dyn Llm>, String> {
    let model = settings.model.as_str();
    match settings.provider.as_str() {
        "anthropic" => {
            let key = api_key.filter(|k| !k.is_empty()).ok_or("Falta la clave API de Anthropic")?;
            let model = if model.is_empty() { ANTHROPIC_DEFAULT_MODEL } else { model };
            Ok(Box::new(AnthropicTranslator::new(http, &key, model)))
        }
        "claudecode" => {
            let exe = find_claude().ok_or(NOT_INSTALLED)?;
            let model = if model.is_empty() { CLAUDE_CODE_MODELS[0] } else { model };
            Ok(Box::new(ClaudeCodeTranslator::new(exe, model, Arc::new(ProcessRunner))))
        }
        other => {
            let base_url = effective_url(other, &settings.base_url);
            if base_url.is_empty() {
                return Err("Falta la URL del servidor".into());
            }
            Ok(Box::new(OpenAICompatibleTranslator::new(http, &base_url, api_key, model)))
        }
    }
}

/// The configured translator plus, when the settings could not be used, why Google
/// is used instead (shown to the user).
pub struct Built {
    pub translator: Box<dyn Translator + Send + Sync>,
    pub issue: Option<String>,
}

fn google_only(http: Arc<dyn HttpClient>) -> Box<dyn Translator + Send + Sync> {
    Box::new(CachedTranslator::new(Box::new(Named::new("Google", Box::new(GoogleTranslator::new(http)))), CACHE_MAX))
}

/// Mirrors `buildTranslator`: Google alone for "google" or without a model;
/// otherwise cache(fallback(llm, Google)). A bad setting (missing key, no server,
/// Claude Code not installed) never stops the app: Google is used and `issue` says why.
pub fn build_translator_with(
    config: &Config,
    http: Arc<dyn HttpClient>,
    api_key: Option<String>,
    find_claude: &dyn Fn() -> Option<PathBuf>,
) -> Built {
    let s = &config.translator;
    if s.provider == "google" || s.model.is_empty() {
        return Built { translator: google_only(http), issue: None };
    }
    let label = provider_label(&s.provider);
    match llm_for(s, api_key, http.clone(), find_claude) {
        Err(e) => Built { translator: google_only(http), issue: Some(format!("{label}: {e}. Se usa Google.")) },
        Ok(llm) => {
            let google: Box<dyn Translator + Send + Sync> = Box::new(GoogleTranslator::new(http));
            let llm: Box<dyn Translator + Send + Sync> = Box::new(LlmTranslator(llm));
            let fallback = FallbackTranslator::new(label, llm, "Google", google);
            Built { translator: Box::new(CachedTranslator::new(Box::new(fallback), CACHE_MAX)), issue: None }
        }
    }
}

/// Real dependencies: ureq, the DPAPI-protected bound key, the installed `claude`.
pub fn build_translator(config: &Config) -> Built {
    let s = &config.translator;
    let key = if s.provider == "claudecode" { None } else { crate::secret::read_api_key(config, &s.provider, &s.base_url) };
    build_translator_with(config, Arc::new(UreqClient::new()), key, &claude_code::find_claude_executable)
}

struct LlmTranslator(Box<dyn Llm>);

impl Translator for LlmTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<crate::core::pipeline::Translation, String> {
        self.0.translate(text, target)
    }
}
