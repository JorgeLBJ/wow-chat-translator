//! Turns :shortcodes: that players type out of Discord/Slack habit (":sob:", ":joy:")
//! into the emoji they stand for, using GitHub's gemoji shortcode list (via the
//! `emojis` crate). Done before translating, so translators see the emoji instead of a
//! word like "sob". Port of `app/src/core/emoji.js`.

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// A letter or a digit; shortcodes touching one (like the 30 in 12:30:45) are left alone.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Replaces known `:name:` shortcodes (case-insensitive) not touching letters or digits.
pub fn replace_shortcodes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    let mut i = 0;
    while i < text.len() {
        let s = &text[i..];
        let c = s.chars().next().expect("non-empty");
        if c == ':' && !prev.is_some_and(is_word_char) {
            let name_len: usize = s[1..].chars().take_while(|&c| is_name_char(c)).map(char::len_utf8).sum();
            let after = &s[1 + name_len..];
            if name_len > 0 && after.starts_with(':') && !after[1..].chars().next().is_some_and(is_word_char) {
                let whole = &s[..name_len + 2];
                let name = s[1..1 + name_len].to_ascii_lowercase();
                match emojis::get_by_shortcode(&name) {
                    Some(emoji) => out.push_str(emoji.as_str()),
                    None => out.push_str(whole),
                }
                prev = Some(':');
                i += whole.len();
                continue;
            }
        }
        out.push(c);
        prev = Some(c);
        i += c.len_utf8();
    }
    out
}
