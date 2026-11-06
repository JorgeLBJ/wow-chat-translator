//! Translation instructions and reply parsing shared by every LLM translator
//! (OpenAI-compatible servers, Anthropic API, Claude Code). Port of `prompt.js`.

use crate::core::pipeline::Translation;

fn lang_name(target: &str) -> &str {
    match target {
        "es" => "Spanish",
        "en" => "English",
        other => other,
    }
}

pub fn system_prompt(target: &str) -> String {
    let lang = lang_name(target);
    [
        format!("You translate World of Warcraft chat messages into {lang}."),
        "Players write short, informal messages full of MMO jargon. Translate the meaning naturally, the way a".into(),
        format!("{lang}-speaking player would write it in chat; do not translate literally."),
        "Common jargon: LFM = looking for more players, LFG = looking for group, WTS = want to sell,".into(),
        "WTB = want to buy, pst = whisper me, inv = invite, ty = thank you, np = no problem, gg = good game,".into(),
        "heal/healer, tank, dps, pally = paladin, lock = warlock, quest = quest/mission, mob = monster.".into(),
        format!("Translate that jargon into the words a {lang}-speaking player would write too (for example, in Spanish,"),
        "LFM -> Buscamos más, WTS -> Vendo, WTB -> Compro, pst -> susurradme). Only keep widely used role words".into(),
        "such as tank, healer or dps, and dungeon or raid abbreviations such as ZG.".into(),
        "Keep player names, item names, zone and dungeon names (e.g. ZG, Deadmines, Brill) as they are.".into(),
        "Tokens like [0], [1], [2] stand for item, spell or quest links: copy them exactly, in a natural position.".into(),
        "The text was read from the screen with OCR and may contain misread characters: lowercase l, 1 and I,".into(),
        "and o and 0, are often confused (for example '101' or 'IoI' is usually 'lol'). Infer the intended words from context.".into(),
        "Keep the tone and emojis. Never add explanations or notes.".into(),
        "Reply in exactly this format and nothing else:".into(),
        "LANG: <ISO 639-1 code of the original message language>".into(),
        "TEXT: <translation>".into(),
    ]
    .join("\n")
}

/// Reads "LANG: xx\nTEXT: ..." (case-insensitive, flexible spacing); anything else
/// is taken whole as the translation with source "auto".
/// Same as the JS regex `^\s*lang:\s*([a-z]{2,3})\s*\n\s*text:\s*([\s\S]*)$`i.
pub fn parse_llm_reply(content: &str) -> Translation {
    let raw = content.trim();
    match parse_format(raw) {
        Some((source, text)) => Translation { text: text.trim().to_string(), source, provider: None, fallback_reason: None },
        None => Translation { text: raw.to_string(), source: "auto".into(), provider: None, fallback_reason: None },
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

fn parse_format(raw: &str) -> Option<(String, &str)> {
    let rest = strip_prefix_ci(raw.trim_start(), "lang:")?;
    // `\s*` after "lang:" may span newlines too.
    let rest = rest.trim_start();
    let code_len = rest.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
    if !(2..=3).contains(&code_len) {
        return None;
    }
    let code = rest[..code_len].to_ascii_lowercase();
    let rest = &rest[code_len..];
    // \s*\n : whitespace that contains at least one newline, then \s* before "text:".
    let ws_len = rest.len() - rest.trim_start().len();
    if !rest[..ws_len].contains('\n') {
        return None;
    }
    let text = strip_prefix_ci(&rest[ws_len..], "text:")?;
    Some((code, text))
}
