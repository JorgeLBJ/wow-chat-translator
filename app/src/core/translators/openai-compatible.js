// Translator for any OpenAI-compatible chat completions API:
// OpenRouter (https://openrouter.ai/api/v1), LM Studio (http://localhost:1234/v1), etc.

const LANG_NAMES = { es: "Spanish", en: "English" };

function systemPrompt(target) {
  const lang = LANG_NAMES[target] ?? target;
  return [
    `You translate World of Warcraft chat messages into ${lang}.`,
    "Players write short, informal messages full of MMO jargon. Translate the meaning naturally, the way a",
    `${lang}-speaking player would write it in chat; do not translate literally.`,
    "Common jargon: LFM = looking for more players, LFG = looking for group, WTS = want to sell,",
    "WTB = want to buy, pst = whisper me, inv = invite, ty = thank you, np = no problem, gg = good game,",
    "heal/healer, tank, dps, pally = paladin, lock = warlock, quest = quest/mission, mob = monster.",
    "Keep player names, item names, zone and dungeon names (e.g. ZG, Deadmines, Brill) as they are.",
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

export function createOpenAICompatibleTranslator({
  baseUrl,
  apiKey = "",
  model,
  fetchFn = globalThis.fetch,
  timeoutMs = 20000,
}) {
  const root = String(baseUrl).replace(/\/+$/, "");
  const headers = () => ({
    "Content-Type": "application/json",
    // Optional attribution headers used by OpenRouter; harmless elsewhere.
    "X-Title": "WoW Chat Translator",
    ...(apiKey ? { Authorization: `Bearer ${apiKey}` } : {}),
  });

  return {
    async translate(text, target) {
      const res = await fetchFn(`${root}/chat/completions`, {
        method: "POST",
        headers: headers(),
        signal: AbortSignal.timeout(timeoutMs),
        body: JSON.stringify({
          model,
          temperature: 0.2,
          max_tokens: 400,
          messages: [
            { role: "system", content: systemPrompt(target) },
            { role: "user", content: text },
          ],
        }),
      });
      if (!res.ok) {
        const detail = typeof res.text === "function" ? (await res.text()).slice(0, 200) : "";
        throw new Error(`LLM HTTP ${res.status}${detail ? `: ${detail}` : ""}`);
      }
      const data = await res.json();
      const parsed = parseLlmReply(data?.choices?.[0]?.message?.content);
      if (!parsed.text) throw new Error("LLM: empty reply");
      return parsed;
    },

    async listModels() {
      const res = await fetchFn(`${root}/models`, { headers: headers(), signal: AbortSignal.timeout(timeoutMs) });
      if (!res.ok) throw new Error(`LLM HTTP ${res.status}`);
      const data = await res.json();
      return (Array.isArray(data?.data) ? data.data : []).map((m) => m.id).filter(Boolean);
    },
  };
}
