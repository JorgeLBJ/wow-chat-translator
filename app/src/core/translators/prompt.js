// Translation instructions and reply parsing shared by every LLM translator
// (OpenAI-compatible servers, Anthropic API, Claude Code).

const LANG_NAMES = { es: "Spanish", en: "English" };

export function systemPrompt(target) {
  const lang = LANG_NAMES[target] ?? target;
  return [
    `You translate World of Warcraft chat messages into ${lang}.`,
    "Players write short, informal messages full of MMO jargon. Translate the meaning naturally, the way a",
    `${lang}-speaking player would write it in chat; do not translate literally.`,
    "Common jargon: LFM = looking for more players, LFG = looking for group, WTS = want to sell,",
    "WTB = want to buy, pst = whisper me, inv = invite, ty = thank you, np = no problem, gg = good game,",
    "heal/healer, tank, dps, pally = paladin, lock = warlock, quest = quest/mission, mob = monster.",
    `Translate that jargon into the words a ${lang}-speaking player would write too (for example, in Spanish,`,
    "LFM -> Buscamos más, WTS -> Vendo, WTB -> Compro, pst -> susurradme). Only keep widely used role words",
    "such as tank, healer or dps, and dungeon or raid abbreviations such as ZG.",
    "Keep player names, item names, zone and dungeon names (e.g. ZG, Deadmines, Brill) as they are.",
    "Tokens like [0], [1], [2] stand for item, spell or quest links: copy them exactly, in a natural position.",
    "Keep the tone and emojis. Never add explanations or notes.",
    "Reply in exactly this format and nothing else:",
    "LANG: <ISO 639-1 code of the original message language>",
    "TEXT: <translation>",
  ].join("\n");
}

export function parseLlmReply(content) {
  const raw = String(content ?? "").trim();
  const match = /^\s*lang:\s*([a-z]{2,3})\s*\n\s*text:\s*([\s\S]*)$/i.exec(raw);
  if (match) return { source: match[1].toLowerCase(), text: match[2].trim() };
  return { source: "auto", text: raw };
}
