//! Maps addon channel labels (English, or localized channel names) to overlay tabs.
//! Port of `app/src/core/channels.js`.

/// Channel labels the addon gives non-player chat (loot, system, NPC speech...), with
/// their Spanish chip text. They go to the "Registro" tab and are never translated.
const LOG_CHANNELS: [(&str, &str); 12] = [
    ("loot", "Botín"),
    ("money", "Oro"),
    ("currency", "Moneda"),
    ("system", "Sistema"),
    ("xp", "XP"),
    ("reputation", "Reputación"),
    ("skill", "Habilidad"),
    ("achievement", "Logro"),
    ("npc", "PNJ"),
    ("bossemote", "Jefe"),
    ("emote", "Emoción"),
    // Lines other addons print straight into the chat frame (Zygor, TomTom...).
    ("addon", "Addon"),
];

/// The tab key of log items.
pub const LOG_TAB: &str = "log";

/// True for the addon's non-player chat labels (*Loot, *System, *NPC...).
pub fn is_log_channel(channel: &str) -> bool {
    log_label(channel).is_some()
}

/// The Spanish chip text of a log channel; `None` for player chat. The addon prefixes
/// log labels with "*" (channel names cannot contain it), so a player-made channel
/// called "Loot" or "System" stays player chat.
pub fn log_label(channel: &str) -> Option<&'static str> {
    let lower = channel.strip_prefix('*')?.to_lowercase();
    LOG_CHANNELS.iter().find(|(key, _)| *key == lower).map(|(_, label)| *label)
}

/// The overlay tab for a channel; `None` means the message only shows under "Todos".
pub fn channel_tab(channel: &str) -> Option<&'static str> {
    match channel.to_lowercase().as_str() {
        "general" => Some("general"),
        "trade" | "comercio" => Some("trade"),
        "guild" | "officer" => Some("guild"),
        "party" | "raid" | "instance" => Some("group"),
        "whisper" => Some("whisper"),
        _ if is_log_channel(channel) => Some(LOG_TAB),
        _ => None,
    }
}
