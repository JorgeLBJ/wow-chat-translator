//! Cheap local check to avoid sending obviously Spanish messages to the translator.
//! It only needs to be right when it says "Spanish"; anything else goes to the
//! translator, which also reports the detected source language.
//! Port of `app/src/core/language.js`.

const ES: &[&str] = &[
    "de", "la", "que", "el", "en", "y", "los", "se", "del", "las", "un", "una", "por", "con", "no", "su", "para",
    "es", "al", "lo", "como", "mas", "más", "pero", "sus", "le", "ya", "este", "esta", "esto", "eso", "esa",
    "porque", "cuando", "muy", "sin", "sobre", "tambien", "también", "me", "hasta", "hay", "donde", "dónde",
    "quien", "quién", "desde", "todo", "todos", "nos", "les", "ni", "otro", "otra", "yo", "tu", "tú", "él",
    "ella", "nosotros", "algo", "alguien", "alguno", "alguna", "necesito", "necesita", "gracias", "hola",
    "vamos", "quiero", "tengo", "tiene", "estoy", "estas", "estás", "bueno", "buena", "aqui", "aquí", "ahora",
    "ayuda", "ayudar", "puedo", "puede", "vendo", "compro", "busco", "grupo", "mazmorra", "tanque", "sanador",
    "favor",
];

const EN: &[&str] = &[
    "the", "a", "an", "is", "are", "you", "i", "to", "and", "of", "in", "for", "on", "it", "this", "that", "with",
    "need", "anyone", "any", "what", "where", "who", "how", "want", "can", "have", "has", "do", "does", "my",
    "your", "we", "they", "lol", "pls", "please", "thanks", "ty", "lf", "lfm", "lfg", "wts", "wtb",
];

/// True when the text is clearly Spanish: an ñ, ¿ or ¡, or at least two common Spanish
/// words and more Spanish than English common words.
pub fn looks_spanish(text: &str) -> bool {
    if text.chars().any(|c| matches!(c, 'ñ' | 'Ñ' | '¿' | '¡')) {
        return true;
    }
    let lower = text.to_lowercase();
    let (mut es, mut en) = (0, 0);
    for word in lower.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()) {
        if ES.contains(&word) {
            es += 1;
        }
        if EN.contains(&word) {
            en += 1;
        }
    }
    es >= 2 && es > en
}
