// Turns new chat messages into overlay items and translates them, unless translation
// is paused. Paused (and failed) messages are kept so the user can translate any one
// of them on demand by clicking it.

import { onlyLinks, protectLinks, restoreLinks, tidyLinks } from "./links.js";
import { fixOcrWords } from "./ocr-fixes.js";
import { looksSpanish } from "./language.js";
import { channelTab } from "./channels.js";

export function createMessagePipeline({ getTranslator, send, isPaused, target = "es", now = Date.now, keep = 300 }) {
  let nextId = 1;
  const pending = new Map(); // id -> { original, masked, links }: not translated yet

  function remember(id, entry) {
    pending.set(id, entry);
    if (pending.size > keep) pending.delete(pending.keys().next().value);
  }

  const inFlight = new Set();

  function translate(id) {
    const entry = pending.get(id);
    inFlight.add(id);
    getTranslator()
      .translate(entry.masked, target)
      .finally(() => inFlight.delete(id))
      .then((res) => {
        pending.delete(id);
        const text = restoreLinks(res.text, entry.links);
        const same = res.source === target || text.trim().toLowerCase() === entry.original.trim().toLowerCase();
        send("chat:update", {
          id,
          lang: res.source === "auto" ? null : res.source,
          translated: same ? null : text,
          provider: res.provider,
          fallbackReason: res.fallbackReason,
          status: "done",
        });
      })
      .catch((err) => send("chat:update", { id, status: "error", error: err.message }));
  }

  return {
    addMessages(messages) {
      for (const msg of messages) {
        const original = fixOcrWords(tidyLinks(msg.text));
        // Links ([Item], [Spell]...) are never translated: mask them, restore after.
        const { text: masked, links } = protectLinks(original);
        const item = {
          id: nextId++,
          time: now(),
          channel: msg.channel,
          tab: channelTab(msg.channel),
          cls: msg.cls,
          player: msg.player,
          original,
          translated: null,
          lang: null,
          status: "pending",
        };
        if (onlyLinks(masked) || looksSpanish(masked)) {
          send("chat:add", { ...item, lang: "es", status: "done" });
          continue;
        }
        remember(item.id, { original, masked, links });
        if (isPaused()) {
          send("chat:add", { ...item, status: "paused" });
          continue;
        }
        send("chat:add", item);
        translate(item.id);
      }
    },

    // Translate one paused or failed message. False when there is nothing to do.
    translateById(id) {
      if (!pending.has(id) || inFlight.has(id)) return false;
      send("chat:update", { id, status: "pending" });
      translate(id);
      return true;
    },
  };
}
