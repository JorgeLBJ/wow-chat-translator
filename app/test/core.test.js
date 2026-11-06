import { describe, it, expect, vi } from "vitest";
import { parseLines } from "../src/core/parser.js";
import { createTracker } from "../src/core/tracker.js";
import { looksSpanish } from "../src/core/language.js";
import { createGoogleTranslator } from "../src/core/translators/google.js";
import { createCachedTranslator } from "../src/core/cache.js";
import { channelTab } from "../src/core/channels.js";
import { createGameWatcher } from "../src/core/game-watcher.js";
import { createPresence } from "../src/core/presence.js";

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
