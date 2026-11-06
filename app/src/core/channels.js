// Maps addon channel labels (English, or localized channel names) to overlay tabs.
// null means the message only shows under "Todos".

const TABS = {
  general: "general",
  trade: "trade",
  comercio: "trade",
  guild: "guild",
  officer: "guild",
  party: "group",
  raid: "group",
  instance: "group",
  whisper: "whisper",
};

export function channelTab(channel) {
  return TABS[String(channel).toLowerCase()] ?? null;
}
