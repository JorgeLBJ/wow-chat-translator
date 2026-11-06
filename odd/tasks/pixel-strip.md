# Pixel strip (replace the OCR panel)

Objective: the addon sends each chat message as a tiny strip of colored cells that the app decodes exactly, so the big black OCR panel and its read errors go away.
Branch: `feat/chat-translator` · Delivery: `ask-on-risk` · Test runner: Vitest (app); Lua checked with luaparse + fengari; live checks in game.

## Specs

- **S1** — User request: "bien vamos con esa adaptación para ya no tener que tener un recuadro negro". The OCR panel is replaced by a small strip of colored cells; the app decodes the exact text (no OCR).
- **S2** — Everything the overlay shows today keeps working: channel, class colors, player, text, item links kept untranslated, pause, click to translate, compose box.
- **S3** — Protocol (own format, design informed by Parley's documented protocol, MIT, https://github.com/Duhlude/parley/blob/main/docs/HOW_IT_WORKS.md, and LibSerpix, MIT):
  - Grid of 64 x 6 cells, each `CELL x CELL` physical pixels (default 2), drawn at the top-left of the WoW window, ignoring UI scale.
  - Each cell carries three 4-bit levels (R, G, B), drawn as `level * 17`.
  - Cells 0-3: marker. Cells 4-19: calibration ramp, cell `4+i` = (i, i, i). Cell 20: sequence number (12 bits). Cell 21: payload length in bytes. Cells 22-23: Fletcher-style checksum (two 12-bit sums mod 4095). Cells 24+: payload, 3 nibbles per cell, 2 nibbles per byte (540 bytes max).
  - Payload: UTF-8 `v1 \x1F channel \x1F CLASS \x1F player \x1F text`; links kept as `[Name]`.
  - Each message stays on screen for `hold` seconds (default 0.1); queued messages follow; the reader skips sequence numbers it already decoded.
- **S4** — Reader: capture only the strip area of the WoW window (~28 times per second), sample each cell center, decode in the app (JS, unit tested).
- **S5** — Safety unchanged: no memory reading, no input automation; replies stay clipboard + manual paste.

## Tasks

| ID | Specs | Task | Route | Status | Commit |
|----|-------|------|-------|--------|--------|
| P1 | S3 | Protocol module in JS: encoder (for tests) + decoder from sampled cell colors (calibration, marker, checksum, UTF-8 fields) | inline | done: 8 round-trip tests (links, accents, emoji, noise, gamma, bad marker, checksum, seq wrap, truncation) GREEN | |
| P2 | S1, S3 | Addon: strip renderer (pixel-exact cells, queue + hold, payload) replacing the OCR panel | inline | done: Lua encoder == JS encoder on all 384 cells (fengari cross-check), JS decodes the Lua frame exactly incl. a real |cnIQ1 link; idle frame; needs in-game check | |
| P3 | S4 | Capture worker: locate the WoW client area, sample cell centers fast, emit raw colors on change | inline | pending | |
| P4 | S2, S4 | App: feed decoded messages into the existing pipeline; retire OCR panel calibration/dock UI | inline | pending | |
| P5 | S1-S5 | Live verification in game (exactness, latency, CPU) | inline | pending | |

## Log

- **L1** (2026-11-06, user, verbatim): "si, aunque me gustaría que averigues en internet si se puede capturar como log el chat, se que lo exploramos pero nuevamente". Research: WoWChatLog.txt is only written at logout (wowprogramming LoggingChat docs, WoWInterface), confirmed again. Found Parley (MIT, pixel strip + desktop overlay, clipboard replies) and LibSerpix (MIT, pixel serialization).
- **L2** (2026-11-06, user, verbatim): "bien vamos con esa adaptación para ya no tener que tener un recuadro negro. Por otro lado, en parley como hacen con el reply solo es copia como el nuestro? o manda al chat". Parley replies: clipboard + manual paste ("You always paste and send replies yourself"). Next: P1.
