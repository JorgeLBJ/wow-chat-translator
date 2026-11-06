//! The translator form of the settings window, without the window: what each provider
//! shows, how switching provider fills the fields, and the save / "Probar" /
//! "Cargar modelos" rules. Port of `settings.js` and the `translator:*` handlers in
//! `app/src/main.js`.
//!
//! The saved API key never comes back to the form: an untouched key field means "keep
//! the saved key", and requests read it only through the binding check
//! (`secret::read_api_key`), so it can never be sent to another provider or server.

use crate::core::pipeline::Translator;
use crate::core::prefs::{Config, TranslatorSettings};
use crate::translators::key_binding::key_binding;
use crate::translators::{effective_url, Llm, PROVIDERS};

/// The sentence "Probar" translates.
pub const TEST_SAMPLE: &str = "LFM ZG need heal, pst";

/// The provider select: id and the label shown in it.
pub const PROVIDER_OPTIONS: [(&str, &str); 6] = [
    ("google", "Google (gratis, sin configuración)"),
    ("openrouter", "OpenRouter (LLM en la nube, con clave)"),
    ("lmstudio", "LM Studio (LLM local)"),
    ("anthropic", "Anthropic / Claude (clave API, créditos de tu plan)"),
    ("claudecode", "Claude Code (tu suscripción de Claude)"),
    ("custom", "Otro servidor compatible con OpenAI"),
];

pub const SAVED_KEY_PLACEHOLDER: &str = "•••••• guardada (escribe para cambiarla)";
pub const NO_KEY_PLACEHOLDER: &str = "sin clave";
pub const MODEL_PLACEHOLDER: &str = "Escribe o carga la lista";
pub const FALLBACK_HINT: &str = "Si el LLM falla o no responde, ese mensaje se traduce con Google.";

pub fn provider_option_label(id: &str) -> &'static str {
    PROVIDER_OPTIONS.iter().find(|(p, _)| *p == id).map_or("", |(_, label)| label)
}

/// URL filled in when switching to a provider (`PRESET_URLS`).
pub fn preset_url(provider: &str) -> &'static str {
    match provider {
        "openrouter" => "https://openrouter.ai/api/v1",
        "lmstudio" => "http://localhost:1234/v1",
        _ => "",
    }
}

/// Model filled in when switching to a provider (`DEFAULT_MODELS`).
pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" | "claudecode" => "claude-haiku-5-5",
        _ => "",
    }
}

pub fn key_hint(provider: &str) -> &'static str {
    match provider {
        "openrouter" => "Créala en openrouter.ai → Keys.",
        "lmstudio" => "No hace falta para LM Studio. Abre LM Studio → Developer → Start Server y carga un modelo.",
        "custom" => "Solo si tu servidor la pide.",
        "anthropic" => {
            "Créala en console.anthropic.com → API Keys. Si esa organización está vinculada a tu plan de Claude, usa sus créditos mensuales."
        }
        "claudecode" => {
            "Sin clave: usa tu sesión de Claude Code (claude auth login) y el cupo de tu plan, el mismo que usas para programar. Tarda unos 3-4 s por mensaje."
        }
        _ => "",
    }
}

/// Google needs no LLM fields at all.
pub fn is_llm(provider: &str) -> bool {
    provider != "google"
}

pub fn shows_url(provider: &str) -> bool {
    !matches!(provider, "anthropic" | "claudecode")
}

pub fn shows_key(provider: &str) -> bool {
    provider != "claudecode"
}

/// The key field: untouched (keep the saved key) or what the user typed.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyInput {
    Untouched,
    Typed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranslatorForm {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub key: KeyInput,
}

impl TranslatorForm {
    /// The form as settings (trimmed, without any key).
    pub fn settings(&self) -> TranslatorSettings {
        TranslatorSettings {
            provider: self.provider.clone(),
            base_url: self.base_url.trim().to_string(),
            model: self.model.trim().to_string(),
            api_key_enc: None,
            key_binding: None,
        }
    }
}

/// What the form knows about the saved translator (`translator:get`); never the key.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedTranslator {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    /// A key is saved and bound to this provider + server.
    pub has_key: bool,
}

pub fn saved_translator(config: &Config) -> SavedTranslator {
    let s = &config.translator;
    let binding = key_binding(&s.provider, &effective_url(&s.provider, &s.base_url));
    let has_key = s.api_key_enc.as_deref().is_some_and(|e| !e.is_empty()) && s.key_binding.as_deref() == Some(&binding);
    SavedTranslator { provider: s.provider.clone(), base_url: s.base_url.clone(), model: s.model.clone(), has_key }
}

/// The fields after choosing `provider` (`syncProvider`): the saved values when it is
/// the saved provider, otherwise its presets; the key field always starts untouched.
pub fn form_for_provider(provider: &str, saved: &SavedTranslator) -> TranslatorForm {
    let same = saved.provider == provider;
    let base_url = if same && !saved.base_url.is_empty() { saved.base_url.clone() } else { preset_url(provider).to_string() };
    let model = if same && !saved.model.is_empty() { saved.model.clone() } else { default_model(provider).to_string() };
    TranslatorForm { provider: provider.to_string(), base_url, model, key: KeyInput::Untouched }
}

pub fn key_placeholder(provider: &str, saved: &SavedTranslator) -> &'static str {
    if saved.provider == provider && saved.has_key {
        SAVED_KEY_PLACEHOLDER
    } else {
        NO_KEY_PLACEHOLDER
    }
}

/// Checked before saving (in the Electron UI).
pub fn validate_form(form: &TranslatorForm) -> Result<(), String> {
    if is_llm(&form.provider) && form.model.trim().is_empty() {
        return Err("Elige un modelo antes de guardar.".into());
    }
    Ok(())
}

/// The settings to save (`translator:set`). Untouched key: the saved key is kept only
/// when provider and server are unchanged; emptied: deleted; typed: encrypted with
/// `encrypt` and bound to this provider + server. Anthropic is refused without a key.
pub fn save_translator(
    prev: &TranslatorSettings,
    form: &TranslatorForm,
    encrypt: &dyn Fn(&str) -> Result<String, String>,
) -> Result<TranslatorSettings, String> {
    if !PROVIDERS.iter().any(|(id, _, _)| *id == form.provider) {
        return Err("Proveedor desconocido".into());
    }
    let mut next = form.settings();
    let binding = key_binding(&next.provider, &effective_url(&next.provider, &next.base_url));
    match &form.key {
        KeyInput::Untouched => {
            let saved = prev.api_key_enc.as_deref().filter(|e| !e.is_empty());
            if let Some(enc) = saved.filter(|_| prev.key_binding.as_deref() == Some(binding.as_str())) {
                next.api_key_enc = Some(enc.to_string());
                next.key_binding = Some(binding);
            }
        }
        KeyInput::Typed(key) => {
            let key = key.trim();
            if !key.is_empty() {
                let enc = encrypt(key).map_err(|_| "Windows no permite cifrar la clave; no se guardó".to_string())?;
                next.api_key_enc = Some(enc);
                next.key_binding = Some(binding);
            }
        }
    }
    if next.provider == "anthropic" && next.api_key_enc.is_none() {
        return Err("Pega tu clave API de Anthropic antes de guardar".into());
    }
    Ok(next)
}

/// The key "Probar" / "Cargar modelos" may send: what was typed, or else the saved key
/// as returned by `read` (production: `secret::read_api_key`, which checks the binding).
/// Claude Code never gets one.
pub fn request_key(
    config: &Config,
    form: &TranslatorForm,
    read: &dyn Fn(&Config, &str, &str) -> Option<String>,
) -> Option<String> {
    if form.provider == "claudecode" {
        return None;
    }
    match &form.key {
        KeyInput::Untouched => read(config, &form.provider, form.base_url.trim()),
        KeyInput::Typed(key) => Some(key.trim().to_string()).filter(|k| !k.is_empty()),
    }
}

/// Builds the LLM for some settings and a key (production: `translators::llm_for`).
pub type MakeLlm<'a> = &'a dyn Fn(&TranslatorSettings, Option<String>) -> Result<Box<dyn Llm>, String>;

/// "Probar": translates [`TEST_SAMPLE`] and returns `«sample» → «translation»`.
pub fn probe(form: &TranslatorForm, key: Option<String>, google: &dyn Translator, make_llm: MakeLlm) -> Result<String, String> {
    let settings = form.settings();
    let result = if !is_llm(&settings.provider) {
        google.translate(TEST_SAMPLE, "es")?
    } else {
        if settings.model.is_empty() {
            return Err("Elige un modelo".into());
        }
        make_llm(&settings, key)?.translate(TEST_SAMPLE, "es")?
    };
    Ok(format!("«{TEST_SAMPLE}» → «{}»", result.text))
}

/// "Cargar modelos": the models the provider offers.
pub fn load_models(form: &TranslatorForm, key: Option<String>, make_llm: MakeLlm) -> Result<Vec<String>, String> {
    make_llm(&form.settings(), key)?.list_models()
}

/// The models shown under the Modelo field: those containing every word of `query`
/// (case-insensitive). An empty query, or one that already names a listed model, shows
/// them all so the user can switch.
pub fn filter_models<'a>(models: &'a [String], query: &str) -> Vec<&'a str> {
    let query = query.trim();
    if query.is_empty() || models.iter().any(|m| m == query) {
        return models.iter().map(String::as_str).collect();
    }
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    models
        .iter()
        .filter(|m| {
            let m = m.to_lowercase();
            words.iter().all(|w| m.contains(w.as_str()))
        })
        .map(String::as_str)
        .collect()
}

pub fn models_message(count: usize) -> String {
    format!("{count} modelos disponibles. Elige uno en la lista ▾ junto al campo Modelo.")
}
