// Translator that runs Claude Code through the official Claude Agent SDK, so it uses
// the Claude subscription the user is logged in with (`claude auth login`) and draws
// from that plan's usage limits. It runs the user's own installed `claude`
// (executablePath), not the large native binary the SDK can bundle. Each message starts a short Claude Code process,
// so it is slower than an API key and queries are capped to a small concurrency.

import { query } from "@anthropic-ai/claude-agent-sdk";
import { parseLlmReply, systemPrompt } from "./prompt.js";

export const CLAUDE_CODE_MODELS = ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"];

// At most `max` tasks run at once. A finishing task hands its slot straight to the
// next waiter, so a new caller can never slip in between and exceed `max`.
function createLimiter(max) {
  let active = 0;
  const waiting = [];
  return async (task) => {
    if (active >= max) await new Promise((resolve) => waiting.push(resolve));
    else active++;
    try {
      return await task();
    } finally {
      const next = waiting.shift();
      if (next) next();
      else active--;
    }
  };
}

function withTimeout(promise, ms) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`Claude Code: sin respuesta tras ${ms / 1000} s (time out)`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

export function createClaudeCodeTranslator({
  model = CLAUDE_CODE_MODELS[0],
  queryFn = query,
  concurrency = 2,
  timeoutMs = 30_000,
  executablePath,
}) {
  const limit = createLimiter(concurrency);

  return {
    translate(text, target) {
      return limit(() => withTimeout(run(text, target), timeoutMs));
    },

    async listModels() {
      return [...CLAUDE_CODE_MODELS];
    },
  };

  async function run(text, target) {
    const messages = queryFn({
      prompt: text,
      options: {
        model,
        systemPrompt: systemPrompt(target),
        tools: [], // pure translation: no file, shell or web access
        settingSources: [], // ignore the user's CLAUDE.md, hooks and settings
        strictMcpConfig: true, // and their MCP servers
        maxTurns: 1,
        ...(executablePath ? { pathToClaudeCodeExecutable: executablePath } : {}),
      },
    });
    let result = null;
    for await (const message of messages) {
      if (message.type === "result") result = message;
    }
    if (!result || result.subtype !== "success" || result.is_error) {
      throw new Error(`Claude Code: ${result?.result || result?.subtype || "no result"}`);
    }
    const parsed = parseLlmReply(result.result);
    if (!parsed.text) throw new Error("Claude Code: empty reply");
    return parsed;
  }
}
