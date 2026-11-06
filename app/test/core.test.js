import { describe, it, expect, vi } from "vitest";
import { parseLines } from "../src/core/parser.js";
import { createTracker } from "../src/core/tracker.js";
import { looksSpanish } from "../src/core/language.js";
import { createGoogleTranslator } from "../src/core/translators/google.js";
import { createCachedTranslator } from "../src/core/cache.js";
import { channelTab } from "../src/core/channels.js";
import { createGameWatcher } from "../src/core/game-watcher.js";
import { createPresence } from "../src/core/presence.js";
import { createOpenAICompatibleTranslator, parseLlmReply } from "../src/core/translators/openai-compatible.js";
import { createFallbackTranslator } from "../src/core/translators/fallback.js";
import { keyBinding } from "../src/core/key-binding.js";
import { protectLinks, restoreLinks, tidyLinks, onlyLinks } from "../src/core/links.js";
import { fixOcrWords } from "../src/core/ocr-fixes.js";
import { createMessagePipeline } from "../src/core/pipeline.js";
import { systemPrompt } from "../src/core/translators/prompt.js";
import { createAnthropicTranslator } from "../src/core/translators/anthropic.js";
import { createClaudeCodeTranslator } from "../src/core/translators/claude-code.js";

describe("parseLines", () => {
  it("parses OCR lines in the format NNN Channel Player Name: message", () => {
    const msgs = parseLines([
      "003 General Moldy Lockrocks: anybody doing maggot eye quest",
      "004 Trade Kaelthas: WTS Arcanite Bar: 40g",
    ]);
    expect(msgs).toEqual([
      { seq: 3, channel: "General", cls: null, player: "Moldy Lockrocks", text: "anybody doing maggot eye quest" },
      { seq: 4, channel: "Trade", cls: null, player: "Kaelthas", text: "WTS Arcanite Bar: 40g" },
    ]);
  });

  it("joins wrapped continuation lines to the previous message", () => {
    const msgs = parseLines([
      "001 General Sunt Chank: i honestly don't know how i quested back then in og tbc without an",
      "adonn",
    ]);
    expect(msgs).toHaveLength(1);
    expect(msgs[0].text).toBe("i honestly don't know how i quested back then in og tbc without an adonn");
  });

  it("drops continuation lines whose first line scrolled out of the panel", () => {
    const msgs = parseLines(["out of view tail", "002 Say Brunhilde: hi"]);
    expect(msgs.map((m) => m.seq)).toEqual([2]);
  });

  it("tolerates OCR confusing digits with letters in the sequence number", () => {
    const msgs = parseLines(["O1l Guild Pedrinho: bora"]);
    expect(msgs[0].seq).toBe(11);
  });

  it("accepts a clipped leading zero read as a parenthesis", () => {
    const msgs = parseLines([")26 Say Daddy Nef: inv", "(27 Say Daddy Nef: wait"]);
    expect(msgs.map((m) => m.seq)).toEqual([26, 27]);
    expect(msgs[0].text).toBe("inv");
  });
});

describe("createTracker", () => {
  const m = (seq, text, player = "Ana") => ({ seq, channel: "Say", player, text });

  it("returns only messages not seen before", () => {
    const t = createTracker();
    expect(t.newMessages([m(1, "hello"), m(2, "world")])).toHaveLength(2);
    expect(t.newMessages([m(1, "hello"), m(2, "world"), m(3, "again")]).map((x) => x.seq)).toEqual([3]);
  });

  it("treats small OCR differences on the same seq as the same message", () => {
    const t = createTracker();
    t.newMessages([m(5, "i think i just used thottbot lol")]);
    expect(t.newMessages([m(5, "i think i iust used thottbot Iol")])).toHaveLength(0);
  });

  it("keeps one message when OCR glued the next, misread line onto it", () => {
    const t = createTracker();
    t.newMessages([m(414, "LF thunderbluff sum")]);
    const glued = m(414, "LF thunderbluff sum 4l5 Trade SHAMAN Ranaghar Foerender: I keep trying to play with a bop-it");
    expect(t.newMessages([glued])).toHaveLength(0);
  });

  it("treats a reused seq with different text as new (addon reloaded)", () => {
    const t = createTracker();
    t.newMessages([m(1, "anybody doing maggot eye quest")]);
    expect(t.newMessages([m(1, "WTS Arcanite Bar 40g pst", "Kael")])).toHaveLength(1);
  });
});

describe("looksSpanish", () => {
  it.each([
    "alguien para Deadmines?",
    "¿alguien sabe dónde está el instructor?",
    "necesito un tanque para la mazmorra",
  ])("detects Spanish: %s", (text) => expect(looksSpanish(text)).toBe(true));

  it.each([
    "anyone know where the trainer is?",
    "bora fazer raid hoje à noite",
    "ich brauche noch Mana, kurz warten",
    "LFM ZG need heal",
  ])("does not flag non-Spanish: %s", (text) => expect(looksSpanish(text)).toBe(false));
});

describe("google translator", () => {
  it("builds the request and returns translation with detected source", async () => {
    const fetchFn = vi.fn(async () => ({
      ok: true,
      json: async () => [["Hola. ¿Cómo estás?", "en"]],
    }));
    const tr = createGoogleTranslator({ fetchFn });
    const res = await tr.translate("Hello. How are you?", "es");
    expect(res).toEqual({ text: "Hola. ¿Cómo estás?", source: "en" });
    const url = new URL(fetchFn.mock.calls[0][0]);
    expect(url.searchParams.get("tl")).toBe("es");
    expect(url.searchParams.get("sl")).toBe("auto");
    expect(url.searchParams.get("q")).toBe("Hello. How are you?");
  });

  it("throws on HTTP errors", async () => {
    const tr = createGoogleTranslator({ fetchFn: async () => ({ ok: false, status: 429 }) });
    await expect(tr.translate("hi", "es")).rejects.toThrow("429");
  });
});

describe("createCachedTranslator", () => {
  it("calls the underlying translator once per text and target", async () => {
    const inner = { translate: vi.fn(async (t) => ({ text: `es:${t}`, source: "en" })) };
    const tr = createCachedTranslator(inner, { max: 10 });
    await tr.translate("ty", "es");
    await tr.translate("ty", "es");
    expect(inner.translate).toHaveBeenCalledTimes(1);
  });

  it("does not cache failures", async () => {
    let fail = true;
    const inner = { translate: vi.fn(async () => { if (fail) throw new Error("x"); return { text: "ok", source: "en" }; }) };
    const tr = createCachedTranslator(inner);
    await expect(tr.translate("a", "es")).rejects.toThrow();
    fail = false;
    await expect(tr.translate("a", "es")).resolves.toEqual({ text: "ok", source: "en" });
  });
});

describe("parseLines with class token", () => {
  it("reads the class between channel and player", () => {
    const [m] = parseLines(["041 General WARLOCK Crystal Keyholder: anyone by riptear"]);
    expect(m).toEqual({ seq: 41, channel: "General", cls: "WARLOCK", player: "Crystal Keyholder", text: "anyone by riptear" });
  });

  it("tolerates OCR errors in the class token", () => {
    expect(parseLines(["042 Say WARL0CK Ana: hi"])[0].cls).toBe("WARLOCK");
    expect(parseLines(["043 Say PALADlN Ana: hi"])[0].cls).toBe("PALADIN");
  });

  it("keeps NOCLASS as unknown class", () => {
    const [m] = parseLines(["044 Say NOCLASS Ana Banana: hi"]);
    expect(m.cls).toBe(null);
    expect(m.player).toBe("Ana Banana");
  });

  it("still parses lines without a class token", () => {
    const [m] = parseLines(["045 General Sunt Chank: hello"]);
    expect(m.cls).toBe(null);
    expect(m.player).toBe("Sunt Chank");
  });
});

describe("channelTab", () => {
  it.each([
    ["General", "general"], ["Trade", "trade"], ["Comercio", "trade"],
    ["Guild", "guild"], ["Officer", "guild"], ["Party", "group"], ["Raid", "group"],
    ["Instance", "group"], ["Whisper", "whisper"], ["Say", null], ["LookingForGroup", null],
  ])("%s -> %s", (channel, tab) => expect(channelTab(channel)).toBe(tab));
});

describe("createGameWatcher", () => {
  const run = async (states) => {
    const onStart = vi.fn();
    const onExit = vi.fn();
    let i = 0;
    const w = createGameWatcher({ isRunning: async () => states[i++], onStart, onExit });
    for (let k = 0; k < states.length; k++) await w.check();
    return { onStart, onExit };
  };

  it("fires onStart when the game appears and onExit when it closes", async () => {
    const { onStart, onExit } = await run([false, true, true, false]);
    expect(onStart).toHaveBeenCalledTimes(1);
    expect(onExit).toHaveBeenCalledTimes(1);
  });

  it("does not fire anything while the game has not started", async () => {
    const { onStart, onExit } = await run([false, false, false]);
    expect(onStart).not.toHaveBeenCalled();
    expect(onExit).not.toHaveBeenCalled();
  });

  it("handles the game being opened and closed several times", async () => {
    const { onStart, onExit } = await run([true, false, true, false]);
    expect(onStart).toHaveBeenCalledTimes(2);
    expect(onExit).toHaveBeenCalledTimes(2);
  });

  it("ignores probe errors instead of treating them as an exit", async () => {
    const onExit = vi.fn();
    const answers = [true, new Error("tasklist failed")];
    let i = 0;
    const w = createGameWatcher({
      isRunning: async () => { const a = answers[i++]; if (a instanceof Error) throw a; return a; },
      onStart: () => {},
      onExit,
    });
    await w.check();
    await w.check();
    expect(onExit).not.toHaveBeenCalled();
  });
});

describe("createPresence", () => {
  const setup = () => {
    const onChange = vi.fn();
    return { p: createPresence({ threshold: 3, onChange }), onChange };
  };

  it("reports the panel as gone only after several consecutive misses", () => {
    const { p, onChange } = setup();
    p.update(true);
    p.update(false);
    p.update(false);
    expect(onChange).toHaveBeenLastCalledWith(true);
    p.update(false);
    expect(onChange).toHaveBeenLastCalledWith(false);
  });

  it("ignores a single flicker", () => {
    const { p, onChange } = setup();
    p.update(true);
    p.update(false);
    p.update(true);
    p.update(false);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("reports the panel back after several consecutive hits", () => {
    const { p, onChange } = setup();
    p.update(true);
    for (let i = 0; i < 3; i++) p.update(false);
    for (let i = 0; i < 3; i++) p.update(true);
    expect(onChange.mock.calls.map((c) => c[0])).toEqual([true, false, true]);
  });
});

describe("parseLlmReply", () => {
  it("reads the LANG and TEXT lines", () => {
    expect(parseLlmReply("LANG: en\nTEXT: Busco más para ZG, falta sanador")).toEqual({
      source: "en", text: "Busco más para ZG, falta sanador",
    });
  });

  it("keeps multi-line translations and tolerates spacing/case", () => {
    expect(parseLlmReply("lang:PT\ntext:  línea uno\nlínea dos")).toEqual({ source: "pt", text: "línea uno\nlínea dos" });
  });

  it("falls back to the whole reply when the format is missing", () => {
    expect(parseLlmReply("  hola a todos  ")).toEqual({ source: "auto", text: "hola a todos" });
  });
});

describe("openai-compatible translator", () => {
  const reply = (content) => ({ ok: true, json: async () => ({ choices: [{ message: { content } }] }) });

  it("posts a chat completion with model, key and target language", async () => {
    const fetchFn = vi.fn(async () => reply("LANG: en\nTEXT: Busco más para ZG"));
    const tr = createOpenAICompatibleTranslator({ baseUrl: "https://openrouter.ai/api/v1/", apiKey: "k-123", model: "m/x", fetchFn });
    const res = await tr.translate("LFM ZG need heal", "es");
    expect(res).toEqual({ source: "en", text: "Busco más para ZG" });
    const [url, init] = fetchFn.mock.calls[0];
    expect(url).toBe("https://openrouter.ai/api/v1/chat/completions");
    expect(init.method).toBe("POST");
    expect(init.headers.Authorization).toBe("Bearer k-123");
    const body = JSON.parse(init.body);
    expect(body.model).toBe("m/x");
    expect(body.messages.at(-1).content).toContain("LFM ZG need heal");
    expect(body.messages[0].content).toContain("Spanish");
  });

  it("omits the Authorization header when no key is set (local servers)", async () => {
    const fetchFn = vi.fn(async () => reply("LANG: en\nTEXT: hola"));
    await createOpenAICompatibleTranslator({ baseUrl: "http://localhost:1234/v1", model: "local", fetchFn }).translate("hi", "es");
    expect(fetchFn.mock.calls[0][1].headers.Authorization).toBeUndefined();
  });

  it("asks for English when the target is en", async () => {
    const fetchFn = vi.fn(async () => reply("LANG: es\nTEXT: anyone for Riptear?"));
    await createOpenAICompatibleTranslator({ baseUrl: "http://x/v1", model: "m", fetchFn }).translate("alguien para Riptear?", "en");
    expect(JSON.parse(fetchFn.mock.calls[0][1].body).messages[0].content).toContain("English");
  });

  it("throws on HTTP errors and empty replies", async () => {
    const bad = createOpenAICompatibleTranslator({ baseUrl: "http://x/v1", model: "m", fetchFn: async () => ({ ok: false, status: 401, text: async () => "no key" }) });
    await expect(bad.translate("hi", "es")).rejects.toThrow("401");
    const empty = createOpenAICompatibleTranslator({ baseUrl: "http://x/v1", model: "m", fetchFn: async () => reply("") });
    await expect(empty.translate("hi", "es")).rejects.toThrow();
  });

  it("lists model ids from /models", async () => {
    const fetchFn = vi.fn(async () => ({ ok: true, json: async () => ({ data: [{ id: "qwen2.5-7b" }, { id: "llama-3.1-8b" }] }) }));
    const tr = createOpenAICompatibleTranslator({ baseUrl: "http://localhost:1234/v1", fetchFn });
    expect(await tr.listModels()).toEqual(["qwen2.5-7b", "llama-3.1-8b"]);
    expect(fetchFn.mock.calls[0][0]).toBe("http://localhost:1234/v1/models");
  });
});

describe("createFallbackTranslator", () => {
  it("uses the primary and tags the result with its name", async () => {
    const tr = createFallbackTranslator(
      { name: "llm", translator: { translate: async () => ({ text: "a", source: "en" }) } },
      { name: "google", translator: { translate: vi.fn() } },
    );
    expect(await tr.translate("x", "es")).toEqual({ text: "a", source: "en", provider: "llm" });
  });

  it("falls back to the secondary when the primary fails", async () => {
    const tr = createFallbackTranslator(
      { name: "llm", translator: { translate: async () => { throw new Error("down"); } } },
      { name: "google", translator: { translate: async () => ({ text: "b", source: "en" }) } },
    );
    expect(await tr.translate("x", "es")).toEqual({ text: "b", source: "en", provider: "google", fallbackReason: "down" });
  });
});

describe("keyBinding", () => {
  it("is the same for equivalent URLs of the same provider", () => {
    expect(keyBinding("openrouter", "https://OpenRouter.ai/api/v1/")).toBe(keyBinding("openrouter", "https://openrouter.ai/api/v1"));
  });

  it("differs when the provider or the server changes", () => {
    const saved = keyBinding("openrouter", "https://openrouter.ai/api/v1");
    expect(keyBinding("custom", "https://openrouter.ai/api/v1")).not.toBe(saved);
    expect(keyBinding("openrouter", "https://evil.example/api/v1")).not.toBe(saved);
    expect(keyBinding("lmstudio", "http://localhost:1234/v1")).not.toBe(saved);
  });
});

describe("item links", () => {
  it("tidies the spaced brackets the addon writes for OCR", () => {
    expect(tidyLinks("wts [ Brazaletes de metal oxidado ] 5g")).toBe("wts [Brazaletes de metal oxidado] 5g");
  });

  it("replaces links with numbered placeholders and restores them unchanged", () => {
    const { text, links } = protectLinks("wts [Arcanite Bar] and [Small Black Pouch] cheap");
    expect(text).toBe("wts [0] and [1] cheap");
    expect(restoreLinks("vendo [0] y [ 1 ] barato", links)).toBe("vendo [Arcanite Bar] y [Small Black Pouch] barato");
  });

  it("leaves unknown placeholder numbers alone", () => {
    expect(restoreLinks("tengo [5]", ["[Bar]"])).toBe("tengo [5]");
  });

  it("detects a message that is only links (nothing to translate)", () => {
    expect(onlyLinks("[0]")).toBe(true);
    expect(onlyLinks(" [0] [1] ! ")).toBe(true);
    expect(onlyLinks("wts [0]")).toBe(false);
  });
});

describe("openai-compatible prompt", () => {
  it("tells the model to keep link placeholders", async () => {
    const fetchFn = vi.fn(async () => ({ ok: true, json: async () => ({ choices: [{ message: { content: "LANG: en\nTEXT: vendo [0]" } }] }) }));
    await createOpenAICompatibleTranslator({ baseUrl: "http://x/v1", model: "m", fetchFn }).translate("wts [0]", "es");
    expect(JSON.parse(fetchFn.mock.calls[0][1].body).messages[0].content).toContain("[0]");
  });
});

describe("tidyLinks with leftover color codes", () => {
  it("removes named-color leftovers like /cnlQ1 that an old addon version leaks", () => {
    expect(tidyLinks("keep them? /cnlQ1 CWAL /cnIQ1: KWYJIBO")).toBe("keep them? CWAL KWYJIBO");
  });
});

describe("anthropic translator", () => {
  const fakeClient = (response) => {
    const create = vi.fn(async () => response);
    return { create, client: { messages: { create }, models: { list: async function* () { yield { id: "claude-haiku-5-5" }; yield { id: "claude-sonnet-5-5" }; } } } };
  };
  const ok = (text) => ({ stop_reason: "end_turn", content: [{ type: "thinking", thinking: "" }, { type: "text", text }] });

  it("sends system prompt, model and low effort, and parses the reply", async () => {
    const { create, client } = fakeClient(ok("LANG: en\nTEXT: Busco más para ZG"));
    const tr = createAnthropicTranslator({ model: "claude-haiku-5-5", client });
    expect(await tr.translate("LFM ZG", "es")).toEqual({ source: "en", text: "Busco más para ZG" });
    const req = create.mock.calls[0][0];
    expect(req.model).toBe("claude-haiku-5-5");
    expect(req.system).toContain("Spanish");
    expect(req.messages).toEqual([{ role: "user", content: "LFM ZG" }]);
    expect(req.output_config).toEqual({ effort: "low" });
    expect(req.temperature).toBeUndefined();
  });

  it("does not send effort to older models that reject it", async () => {
    const { create, client } = fakeClient(ok("LANG: en\nTEXT: hola"));
    await createAnthropicTranslator({ model: "claude-haiku-4-5", client }).translate("hi", "es");
    expect(create.mock.calls[0][0].output_config).toBeUndefined();
  });

  it("throws on refusals so the Google fallback takes over", async () => {
    const { client } = fakeClient({ stop_reason: "refusal", content: [] });
    await expect(createAnthropicTranslator({ model: "claude-haiku-5-5", client }).translate("x", "es")).rejects.toThrow();
  });

  it("lists models", async () => {
    const { client } = fakeClient(ok(""));
    expect(await createAnthropicTranslator({ model: "m", client }).listModels()).toEqual(["claude-haiku-5-5", "claude-sonnet-5-5"]);
  });
});

describe("claude code translator", () => {
  const resultMsg = (extra) => ({ type: "result", subtype: "success", is_error: false, result: "LANG: en\nTEXT: hola", ...extra });

  it("runs a tool-less, settings-less single-turn query with the chosen model", async () => {
    const queryFn = vi.fn(async function* () { yield { type: "system" }; yield resultMsg(); });
    const tr = createClaudeCodeTranslator({ model: "claude-haiku-5-5", queryFn });
    expect(await tr.translate("hi", "es")).toEqual({ source: "en", text: "hola" });
    const { prompt, options } = queryFn.mock.calls[0][0];
    expect(prompt).toBe("hi");
    expect(options).toMatchObject({ model: "claude-haiku-5-5", tools: [], settingSources: [], maxTurns: 1, strictMcpConfig: true });
    expect(options.systemPrompt).toContain("Spanish");
  });

  it("throws when Claude Code reports an error", async () => {
    const queryFn = async function* () { yield resultMsg({ is_error: true, result: "rate limited" }); };
    await expect(createClaudeCodeTranslator({ model: "m", queryFn }).translate("hi", "es")).rejects.toThrow("rate limited");
    const noResult = async function* () { yield { type: "result", subtype: "error_max_turns", is_error: true }; };
    await expect(createClaudeCodeTranslator({ model: "m", queryFn: noResult }).translate("hi", "es")).rejects.toThrow();
  });

  it("runs at most `concurrency` queries at once", async () => {
    let running = 0;
    let peak = 0;
    const queryFn = async function* () {
      running++;
      peak = Math.max(peak, running);
      await new Promise((r) => setTimeout(r, 20));
      running--;
      yield resultMsg();
    };
    const tr = createClaudeCodeTranslator({ model: "m", queryFn, concurrency: 2 });
    await Promise.all([1, 2, 3, 4, 5].map(() => tr.translate("hi", "es")));
    expect(peak).toBe(2);
  });
});

describe("claude code translator timeout", () => {
  it("rejects when Claude Code does not answer in time, so the fallback can run", async () => {
    const queryFn = async function* () { await new Promise((r) => setTimeout(r, 200)); yield { type: "result", subtype: "success", is_error: false, result: "x" }; };
    await expect(createClaudeCodeTranslator({ model: "m", queryFn, timeoutMs: 30 }).translate("hi", "es")).rejects.toThrow(/tiempo|time/i);
  });
});

describe("claude code translator executable", () => {
  it("runs the user's installed Claude Code when a path is given", async () => {
    const queryFn = vi.fn(async function* () { yield { type: "result", subtype: "success", is_error: false, result: "LANG: en\nTEXT: hola" }; });
    const exe = "C:/Users/x/.local/bin/claude.exe";
    await createClaudeCodeTranslator({ model: "m", queryFn, executablePath: exe }).translate("hi", "es");
    expect(queryFn.mock.calls[0][0].options.pathToClaudeCodeExecutable).toBe(exe);
  });
});

describe("item links written with < > by the addon", () => {
  it("reads < Name > links, including several in a row, as [Name]", () => {
    expect(tidyLinks("WTS < Runed Copper Bracers > < Heavy Copper Maul > cheap")).toBe("WTS [Runed Copper Bracers] [Heavy Copper Maul] cheap");
  });

  it("keeps links glued without a space apart", () => {
    expect(tidyLinks("WTS < A Bar >< B Pouch >")).toBe("WTS [A Bar][B Pouch]");
  });

  it("does not treat <3 or comparisons typed by players as links", () => {
    expect(tidyLinks("i <3 this guild > all")).toBe("i <3 this guild > all");
  });

  it("protects < > links from translation end to end", () => {
    const { text, links } = protectLinks(tidyLinks("anyone selling < Perfect Deviate Scale >?"));
    expect(text).toBe("anyone selling [0]?");
    expect(restoreLinks("¿alguien vende [0]?", links)).toBe("¿alguien vende [Perfect Deviate Scale]?");
  });
});

describe("fixOcrWords", () => {
  it.each([
    ["101", "lol"],
    ["1o1", "lol"],
    ["IoI", "lol"],
    ["l0l", "lol"],
    ["haha 1o1 nice", "haha lol nice"],
    ["101!", "lol!"],
  ])("reads the standalone OCR misread %s as lol", (input, expected) => {
    expect(fixOcrWords(input)).toBe(expected);
  });

  it.each(["WTS for 101g", "level 101 boss", "haha 101 nice", "1010", "room 101b", "lol"])("leaves %s unchanged", (text) => {
    expect(fixOcrWords(text)).toBe(text);
  });
});

describe("system prompt OCR hint", () => {
  it("tells the LLM that the text comes from OCR and may confuse l/1 and o/0", () => {
    expect(systemPrompt("es")).toMatch(/OCR/);
  });
});

describe("claude code translator abort", () => {
  it("aborts the Claude Code query when it times out, so the process does not linger", async () => {
    let controller;
    const queryFn = async function* ({ options }) {
      controller = options.abortController;
      await new Promise((r) => setTimeout(r, 200));
      yield { type: "result", subtype: "success", is_error: false, result: "x" };
    };
    await expect(createClaudeCodeTranslator({ model: "m", queryFn, timeoutMs: 20 }).translate("hi", "es")).rejects.toThrow();
    expect(controller).toBeInstanceOf(AbortController);
    expect(controller.signal.aborted).toBe(true);
  });

  it("does not abort a query that finishes in time", async () => {
    let controller;
    const queryFn = async function* ({ options }) {
      controller = options.abortController;
      yield { type: "result", subtype: "success", is_error: false, result: "LANG: en\nTEXT: hola" };
    };
    await createClaudeCodeTranslator({ model: "m", queryFn, timeoutMs: 1000 }).translate("hi", "es");
    expect(controller.signal.aborted).toBe(false);
  });
});

describe("message pipeline (pause + translate on demand)", () => {
  const msg = (seq, text) => ({ seq, channel: "Trade", cls: "MAGE", player: "Ana", text });
  const setup = ({ paused = false, translate } = {}) => {
    const sent = [];
    const tr = { translate: vi.fn(translate ?? (async (t) => ({ text: `ES(${t})`, source: "en", provider: "X" }))) };
    let isPaused = paused;
    const p = createMessagePipeline({ getTranslator: () => tr, send: (ch, data) => sent.push([ch, data]), isPaused: () => isPaused });
    return { p, tr, sent, setPaused: (v) => { isPaused = v; } };
  };
  const flush = () => new Promise((r) => setTimeout(r, 0));

  it("translates new messages and keeps links untranslated", async () => {
    const { p, sent } = setup();
    p.addMessages([msg(1, "wts [Arcanite Bar] cheap")]);
    await flush();
    expect(sent[0]).toEqual(["chat:add", expect.objectContaining({ id: 1, status: "pending", original: "wts [Arcanite Bar] cheap" })]);
    expect(sent[1]).toEqual(["chat:update", expect.objectContaining({ id: 1, status: "done", translated: "ES(wts [Arcanite Bar] cheap)" })]);
  });

  it("does not translate while paused and marks the message as paused", async () => {
    const { p, tr, sent } = setup({ paused: true });
    p.addMessages([msg(1, "anyone know where the trainer is?")]);
    await flush();
    expect(tr.translate).not.toHaveBeenCalled();
    expect(sent).toEqual([["chat:add", expect.objectContaining({ id: 1, status: "paused" })]]);
  });

  it("translates one paused message on demand", async () => {
    const { p, tr, sent } = setup({ paused: true });
    p.addMessages([msg(1, "need a tank"), msg(2, "lf healer")]);
    expect(p.translateById(2)).toBe(true);
    await flush();
    expect(tr.translate).toHaveBeenCalledTimes(1);
    expect(sent.slice(2)).toEqual([
      ["chat:update", { id: 2, status: "pending" }],
      ["chat:update", expect.objectContaining({ id: 2, status: "done", translated: "ES(lf healer)" })],
    ]);
  });

  it("only translates messages that arrive after resuming", async () => {
    const { p, tr, setPaused } = setup({ paused: true });
    p.addMessages([msg(1, "need a tank")]);
    setPaused(false);
    p.addMessages([msg(2, "lf healer")]);
    await flush();
    expect(tr.translate).toHaveBeenCalledTimes(1);
    expect(tr.translate.mock.calls[0][0]).toBe("lf healer");
  });

  it("does not send Spanish or link-only messages to the translator, paused or not", async () => {
    const { p, tr, sent } = setup({ paused: true });
    p.addMessages([msg(1, "alguien para la mazmorra?"), msg(2, "[Arcanite Bar]")]);
    await flush();
    expect(tr.translate).not.toHaveBeenCalled();
    expect(sent.map(([, d]) => d.status)).toEqual(["done", "done"]);
  });

  it("lets a failed translation be retried on demand", async () => {
    let fail = true;
    const { p, sent } = setup({ translate: async (t) => { if (fail) throw new Error("down"); return { text: "ok", source: "en" }; } });
    p.addMessages([msg(1, "hello there")]);
    await flush();
    expect(sent.at(-1)).toEqual(["chat:update", { id: 1, status: "error", error: "down" }]);
    fail = false;
    expect(p.translateById(1)).toBe(true);
    await flush();
    expect(sent.at(-1)).toEqual(["chat:update", expect.objectContaining({ id: 1, status: "done", translated: "ok" })]);
  });

  it("returns false for unknown or already translated messages", async () => {
    const { p } = setup();
    p.addMessages([msg(1, "hello there")]);
    await flush();
    expect(p.translateById(1)).toBe(false);
    expect(p.translateById(99)).toBe(false);
  });
});

describe("message pipeline double click", () => {
  it("ignores a second request while the same message is still being translated", async () => {
    const sent = [];
    let release;
    const tr = { translate: vi.fn(() => new Promise((r) => { release = () => r({ text: "ok", source: "en" }); })) };
    const p = createMessagePipeline({ getTranslator: () => tr, send: (c, d) => sent.push([c, d]), isPaused: () => true });
    p.addMessages([{ seq: 1, channel: "Say", cls: null, player: "A", text: "hello there" }]);
    expect(p.translateById(1)).toBe(true);
    expect(p.translateById(1)).toBe(false);
    release();
    await new Promise((r) => setTimeout(r, 0));
    expect(tr.translate).toHaveBeenCalledTimes(1);
  });
});

describe("message pipeline with exact (strip) text", () => {
  it("does not apply OCR corrections when the text is exact", async () => {
    const sent = [];
    const tr = { translate: vi.fn(async (t) => ({ text: t, source: "en" })) };
    const p = createMessagePipeline({ getTranslator: () => tr, send: (c, d) => sent.push([c, d]), isPaused: () => false, ocrFixes: false });
    p.addMessages([{ channel: "Trade", cls: "MAGE", player: "Ana", text: "101" }, { channel: "Say", cls: null, player: "B", text: "i < 3 > you" }]);
    expect(sent.filter(([c]) => c === "chat:add").map(([, d]) => d.original)).toEqual(["101", "i < 3 > you"]);
  });
});

describe("message pipeline: only real links are kept untranslated (strip)", () => {
  it("masks links marked by the addon and translates brackets typed by players", async () => {
    const sent = [];
    const tr = { translate: vi.fn(async (t) => ({ text: `ES(${t})`, source: "en" })) };
    const p = createMessagePipeline({ getTranslator: () => tr, send: (c, d) => sent.push([c, d]), isPaused: () => false, ocrFixes: false });
    p.addMessages([{ channel: "Trade", cls: "MAGE", player: "Ana", text: "wts \u0002[Arcanite Bar]\u0003 and [cheap stuff] pst" }]);
    await new Promise((r) => setTimeout(r, 0));
    expect(tr.translate.mock.calls[0][0]).toBe("wts [0] and [cheap stuff] pst");
    const add = sent.find(([c]) => c === "chat:add")[1];
    expect(add.original).toBe("wts [Arcanite Bar] and [cheap stuff] pst");
    const upd = sent.find(([c]) => c === "chat:update")[1];
    expect(upd.translated).toBe("ES(wts [Arcanite Bar] and [cheap stuff] pst)");
  });

  it("treats a message made only of real links as nothing to translate", async () => {
    const sent = [];
    const tr = { translate: vi.fn() };
    const p = createMessagePipeline({ getTranslator: () => tr, send: (c, d) => sent.push([c, d]), isPaused: () => false, ocrFixes: false });
    p.addMessages([{ channel: "Trade", cls: null, player: "B", text: "\u0002[Linen Cloth]\u0003" }]);
    expect(tr.translate).not.toHaveBeenCalled();
    expect(sent[0][1]).toMatchObject({ original: "[Linen Cloth]", status: "done" });
  });
});
