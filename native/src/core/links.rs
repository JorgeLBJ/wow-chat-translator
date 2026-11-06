//! Item, spell, quest... links must never be translated. The addon wraps real WoW links
//! (`|H...|h[Name]|h`) in \x02 ... \x03, characters players cannot type; only those are
//! swapped for numbered placeholders (`[0]`, `[1]`...) before translating and put back
//! after. Brackets a player typed by hand are ordinary text.
//! Port of the strip path of `app/src/core/links.js`.

const START: char = '\u{2}';
const END: char = '\u{3}';

/// Removes every link marker, leaving the link text as shown in game.
pub fn strip_link_markers(text: &str) -> String {
    text.chars().filter(|&c| c != START && c != END).collect()
}

/// Replaces each marked link (`\x02name\x03`) with `[n]` and returns the masked text
/// (without stray markers) and the link texts in order.
pub fn protect_marked_links(text: &str) -> (String, Vec<String>) {
    let mut links = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(START) {
        out.push_str(&rest[..start]);
        let after = &rest[start + START.len_utf8()..];
        match after.find([START, END]) {
            // A complete \x02...\x03 with no marker inside.
            Some(end) if after[end..].starts_with(END) => {
                links.push(after[..end].to_string());
                out.push_str(&format!("[{}]", links.len() - 1));
                rest = &after[end + END.len_utf8()..];
            }
            // Unmatched start marker: dropped, scanning goes on after it.
            _ => rest = after,
        }
    }
    out.push_str(rest);
    (strip_link_markers(&out), links)
}

/// Parses a placeholder `[ n ]` (whitespace around the digits allowed) at the start of
/// `s`; returns the index text and the byte length of the whole placeholder.
fn placeholder_at(s: &str) -> Option<(&str, usize)> {
    let inner = s.strip_prefix('[')?;
    let body = inner.trim_start();
    let digits_len = body.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    let digits = &body[..digits_len];
    let tail = body[digits_len..].trim_start();
    tail.strip_prefix(']')?;
    let len = s.len() - tail.len() + 1;
    Some((digits, len))
}

/// Puts the links back in place of `[n]` placeholders (translators may add spaces,
/// as in `[ 1 ]`). Placeholders with no matching link are left as they are.
pub fn restore_links(text: &str, links: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let s = &text[i..];
        if let Some((digits, len)) = placeholder_at(s) {
            if let Some(link) = digits.parse::<usize>().ok().and_then(|n| links.get(n)) {
                out.push_str(link);
            } else {
                out.push_str(&s[..len]);
            }
            i += len;
        } else {
            let c = s.chars().next().expect("non-empty");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Removes every placeholder from `text`.
fn without_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let s = &text[i..];
        if let Some((_, len)) = placeholder_at(s) {
            i += len;
        } else {
            let c = s.chars().next().expect("non-empty");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// True when nothing but placeholders and punctuation is left to translate.
pub fn only_links(masked: &str) -> bool {
    !without_placeholders(masked).chars().any(char::is_alphabetic)
}
