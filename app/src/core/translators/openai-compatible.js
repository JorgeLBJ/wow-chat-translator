// Translator for any OpenAI-compatible chat completions API:
// OpenRouter (https://openrouter.ai/api/v1), LM Studio (http://localhost:1234/v1), etc.

import { parseLlmReply, systemPrompt } from "./prompt.js";

export { parseLlmReply };

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
