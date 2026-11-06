//! The compose box: outgoing messages are translated (Spanish passes through) and
//! copied to the clipboard; the user pastes them in WoW. Port of `compose:translate`
//! in `app/src/main.js` and the compose part of `overlay.js`.

use crate::core::pipeline::Translator;

pub const PLACEHOLDER: &str = "Escribe en español… Enter = traducir y copiar";
pub const TRANSLATING: &str = "Traduciendo…";

/// Languages offered by the EN/ES toggle: (code, label).
pub const OUT_LANGS: [(&str, &str); 2] = [("en", "EN"), ("es", "ES")];

/// The text to copy: `text` trimmed, translated to `lang` unless it is "es".
pub fn translate_outgoing(text: &str, lang: &str, translator: &dyn Translator) -> Result<String, String> {
    let clean = text.trim();
    if clean.is_empty() {
        return Err("Mensaje vacío".into());
    }
    if lang == "es" {
        return Ok(clean.to_string());
    }
    translator.translate(clean, lang).map(|t| t.text)
}

pub fn copied_message(text: &str) -> String {
    format!("✓ Copiado: «{text}». En WoW: Enter → Ctrl+V → Enter")
}

pub fn failed_message(error: &str) -> String {
    format!("No se pudo traducir: {error}")
}
