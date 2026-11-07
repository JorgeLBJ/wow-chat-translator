//! Item, spell, quest... links must never be translated. The addon wraps real WoW links
//! (`|H...|h[Name]|h`) in \x02 ... \x03, characters players cannot type; only those are
//! swapped for numbered placeholders (`[0]`, `[1]`...) before translating and put back
//! after. Brackets a player typed by hand are ordinary text.
//!
//! Since addon 0.3.0 a link may also carry its type and first id after a \x04, as in
//! `\x02[Arcanite Bar]\x04item:12360\x03`. That ref is never shown nor translated;
//! [`marked_link_refs`] exposes it. The old form `\x02[Name]\x03` still decodes.
//! Port of the strip path of `app/src/core/links.js`.

const START: char = '\u{2}';
const END: char = '\u{3}';
const REF: char = '\u{4}';

/// A link's type and first id (`item` and `12360` for an item link).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRef {
    /// The link text as shown in game, brackets included ("[Arcanite Bar]").
    pub name: String,
    pub kind: String,
    pub id: u64,
}

/// Removes every link marker and link ref, leaving the link text as shown in game.
pub fn strip_link_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ref = false;
    for c in text.chars() {
        match c {
            REF => in_ref = true,
            START | END => in_ref = false,
            _ if !in_ref => out.push(c),
            _ => {}
        }
    }
    out
}

/// Replaces each complete marked link (`\x02body\x03`, no marker inside) with `[n]`;
/// returns the masked text (markers not yet stripped) and the raw link bodies in order.
fn mask(text: &str) -> (String, Vec<&str>) {
    let mut bodies = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(START) {
        out.push_str(&rest[..start]);
        let after = &rest[start + START.len_utf8()..];
        match after.find([START, END]) {
            Some(end) if after[end..].starts_with(END) => {
                bodies.push(&after[..end]);
                out.push_str(&format!("[{}]", bodies.len() - 1));
                rest = &after[end + END.len_utf8()..];
            }
            // Unmatched start marker: dropped, scanning goes on after it.
            _ => rest = after,
        }
    }
    out.push_str(rest);
    (out, bodies)
}

/// A link body's shown name and its ref text, if any.
fn split_ref(body: &str) -> (&str, Option<&str>) {
    match body.split_once(REF) {
        Some((name, r)) => (name, Some(r)),
        None => (body, None),
    }
}

/// Replaces each marked link (`\x02name\x03`) with `[n]` and returns the masked text
/// (without stray markers) and the link texts in order (without their refs).
pub fn protect_marked_links(text: &str) -> (String, Vec<String>) {
    let (out, bodies) = mask(text);
    let links = bodies.into_iter().map(|b| split_ref(b).0.to_string()).collect();
    (strip_link_markers(&out), links)
}

/// The refs of the marked links that carry a valid one (`type:number`), in order.
pub fn marked_link_refs(text: &str) -> Vec<LinkRef> {
    mask(text)
        .1
        .into_iter()
        .filter_map(|body| {
            let (name, r) = split_ref(body);
            let (kind, id) = r?.split_once(':')?;
            if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            Some(LinkRef { name: name.to_string(), kind: kind.to_string(), id: id.parse().ok()? })
        })
        .collect()
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
