// Translator using the Anthropic API (official @anthropic-ai/sdk) with a Console
// API key, e.g. one from the organization linked to a Max plan's monthly credits.

import Anthropic from "@anthropic-ai/sdk";
import { parseLlmReply, systemPrompt } from "./prompt.js";

export const ANTHROPIC_DEFAULT_MODEL = "claude-haiku-5-5";

// output_config.effort exists on the Claude 5 family; older models (e.g. Haiku 4.5)
// return a 400 when it is sent.
const supportsEffort = (model) => /^claude-(fable|mythos|opus|sonnet|haiku)-5/.test(model);

export function createAnthropicTranslator({ apiKey, model = ANTHROPIC_DEFAULT_MODEL, client }) {
  const api = client ?? new Anthropic({ apiKey, maxRetries: 1, timeout: 20_000 });

  return {
    async translate(text, target) {
      const response = await api.messages.create({
        model,
        max_tokens: 1024,
        system: systemPrompt(target),
        messages: [{ role: "user", content: text }],
        // Short chat lines need no deep reasoning; low effort keeps it fast and cheap.
        ...(supportsEffort(model) ? { output_config: { effort: "low" } } : {}),
      });
      if (response.stop_reason === "refusal") throw new Error("Claude declined to translate this message");
      const reply = response.content
        .filter((block) => block.type === "text")
        .map((block) => block.text)
        .join("");
      const parsed = parseLlmReply(reply);
      if (!parsed.text) throw new Error("Claude: empty reply");
      return parsed;
    },

    async listModels() {
      const ids = [];
      for await (const m of api.models.list()) ids.push(m.id);
      return ids;
    },
  };
}
