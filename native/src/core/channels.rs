//! Maps addon channel labels (English, or localized channel names) to overlay tabs.
//! Port of `app/src/core/channels.js`.

/// The overlay tab for a channel; `None` means the message only shows under "Todos".
pub fn channel_tab(channel: &str) -> Option<&'static str> {
    match channel.to_lowercase().as_str() {
        "general" => Some("general"),
        "trade" | "comercio" => Some("trade"),
        "guild" | "officer" => Some("guild"),
        "party" | "raid" | "instance" => Some("group"),
        "whisper" => Some("whisper"),
        _ => None,
    }
}
