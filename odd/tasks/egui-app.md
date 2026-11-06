# Native app (Rust + egui)

Objective: rewrite the companion app in Rust + egui (single process, ~3x less memory than Electron), reading the same addon pixel strip; Electron (`app/`) keeps working on `feat/chat-translator` until parity.
Branch: `feat/egui-app` (worktree `../wow-chat-translator-worktrees/egui`), new crate in `native/`; addon in `addon/` is shared. Delivery: `ask-on-risk`. Tests: `cargo test` (native) + Vitest (Electron, unchanged).

## Specs

- **S1** — User, verbatim: "ok, si, realizala en otro repo, o como gestionaremos eso" (answer to "¿Arrancamos la reescritura completa en egui?"). Decision: same repo, `native/` folder, branch `feat/egui-app`; Tauri branch parked (not merged).
- **S2** — Reference prototype (branch `feat/tauri-migration`, `prototypes/egui-chat`): eframe 0.33 + egui-twemoji 0.8 (pins egui 0.33); measured 73-75 MB private, 1 process.
- **S3** — Feature parity with the Electron app (source of truth: `app/src`): overlay (tabs Todos/General/Comercio/Hermandad/Grupo/Susurros with unread counts, class colors, channel tags, translated/original toggle, hover original, pause + click to translate, compose box with EN/ES, clipboard + bring WoW to front, Ctrl+Shift+Enter only while WoW runs), settings window (translator providers Google / OpenRouter / LM Studio / custom OpenAI-compatible / Anthropic API / Claude Code via installed `claude`; API key encrypted at rest and bound to provider+URL; Probar / Cargar modelos; window prefs; start with Windows; clear; quit), tray, follows Wow.exe (show/hide), hides when the strip is absent, emoji shortcodes, links kept untranslated (only addon-marked \x02[..]\x03), prefs persisted.
- **S4** — Pixel-strip protocol must match `app/src/core/strip.js` + `strip-reader.js` exactly (layout, calibration fit, checksum, idle frame, layout search cell 1-4 / offset 0-2); capture like `app/ocr/strip-worker.ps1` (260x28 patch at the WoW client-area origin every ~35 ms, marker precheck) but in-process (no PowerShell).
- **S5** — Safety unchanged: no memory reading, no input automation, replies via clipboard + manual paste.
- **S6** — Twemoji graphics are CC-BY 4.0: credit in an About section.

## Tasks

| ID | Specs | Task | Route | Status | Commit |
|----|-------|------|-------|--------|--------|
| E1 | S2, S4 | `native/` crate: protocol (encode for tests, decode with quadratic calibration fit), strip locator, Win32 capture of the WoW client patch, game-process check; unit tests ported from `app/test/strip.test.js`; a debug binary that prints decoded messages | delegated (context backstop) | done: 15 tests GREEN incl. golden frame identical to JS (384 cells); strip-debug runs; live decode pending (WoW covered by another window during checks) | |
| E2 | S3 | Core: message pipeline (links, emoji shortcodes, pause/translate-by-id, Spanish heuristic, tabs), prefs/config JSON | delegated (context backstop) | done: 27 core tests + 15 strip tests GREEN; event/job driven pipeline (no callbacks) | |
| E3 | S3, S5 | Translators: Google, OpenAI-compatible, Anthropic (HTTP), Claude Code (installed CLI), cache + Google fallback; DPAPI-encrypted bound key | delegated (context backstop) | done: 45 tests GREEN; live Google 409 ms and Claude Code 4.21 s; security verifier pending (risk item 2) | |
| E4 | S3 | Overlay UI in egui (+ tray, autostart, always on top, capture exclusion) | pending | pending | |
| E5 | S3 | Settings window, compose (clipboard + focus WoW) | pending | pending | |
| E6 | S1-S6 | Parity check in game, memory/CPU, installer | pending | pending | |

## Log

- **L1** (2026-11-06, user, verbatim): "si, vamos con egui" (prototype) then "ok, si, realizala en otro repo, o como gestionaremos eso". Tauri worktree removed (2.9 GB, branch `feat/tauri-migration` kept); egui worktree created from `feat/chat-translator` at 2312a03. Parent context is past the long-session backstop: implementation units are delegated to bounded writers.
- **L2** (2026-11-06): E1 delivered by a bounded writer (windows 0.62.2; RED 15/15 failing stubs, then GREEN 15/15). Parent spot check: `cargo test` 15 passed incl. `matches_js_golden_frame`. Live run printed "wow process running: true / no strip visible"; investigation: the strip was drawn in WoW (PrintWindow) but a maximized WebStorm window covered the screen (WindowFromPoint at (5,5) = webstorm64), and the Electron reader also logged "strip absent": correct behavior, live decode still to verify with WoW in front. Risk: item 6 (BitBlt/DPI capture only checked by live runs).
- **L3** (2026-11-06): E2 delivered by a bounded writer: core/{links,emoji,language,channels,pipeline,prefs}.rs; crates emojis 0.9.1 ((MIT OR Apache-2.0) AND Unicode-3.0, permissive), serde 1.0.229, serde_json 1.0.154. RED (module missing, compile error) then GREEN 27/27 (+15 E1). Parent spot check: cargo test green. Deviations noted: char::is_alphabetic vs JS \p{L} on rare chars; a wrongly typed config field resets the whole file to defaults; save() drops unknown keys (fine once native replaces Electron; both apps must not share one config.json meanwhile).
- **L4** (2026-11-06): E3 delivered by a bounded writer: ureq 3.5.1 (rustls), base64 0.23.1, windows Win32_Security(+Cryptography) for DPAPI; anthropic-version 2026-10-21 (from the Claude API skill), no temperature; Claude Code via `claude -p --model --system-prompt --tools "" --setting-sources "" --strict-mcp-config --disable-slash-commands --no-session-persistence --output-format json`, prompt on stdin, hidden window, 30 s kill. RED (13 compile errors) then GREEN 45/45 (+42). Electron-saved keys are not DPAPI-compatible: re-enter once.
- **L5** (2026-11-06): Lesson from the Electron app (its L44): the overlay must follow strip presence through a gate (show at once, hide only after ~2 s of continuous absence, never while focused), otherwise focusing the overlay over a fullscreen WoW causes a hide/show loop that freezes the screen. Apply in E4.
