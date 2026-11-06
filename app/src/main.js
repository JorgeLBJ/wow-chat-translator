import { app, BrowserWindow, Menu, Tray, clipboard, globalShortcut, ipcMain, nativeImage, safeStorage, screen } from "electron";
import { execFile, execFileSync, spawn } from "node:child_process";
import { createInterface } from "node:readline";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createMessagePipeline } from "./core/pipeline.js";
import { findStrip } from "./core/strip-reader.js";
import { createGameWatcher } from "./core/game-watcher.js";
import { createGoogleTranslator } from "./core/translators/google.js";
import { createCachedTranslator } from "./core/cache.js";
import { createOpenAICompatibleTranslator } from "./core/translators/openai-compatible.js";
import { createFallbackTranslator } from "./core/translators/fallback.js";
import { ANTHROPIC_DEFAULT_MODEL, createAnthropicTranslator } from "./core/translators/anthropic.js";
import { CLAUDE_CODE_MODELS, createClaudeCodeTranslator } from "./core/translators/claude-code.js";
import { keyBinding } from "./core/key-binding.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const STRIP_WORKER = path.join(here, "..", "ocr", "strip-worker.ps1");
const FOCUS_WOW = path.join(here, "..", "ocr", "focus-wow.ps1");
const DETECT_PANEL = path.join(here, "..", "ocr", "detect-panel.ps1");
const TARGET_LANG = "es";
const GAME_EXE = "Wow.exe";
const GAME_CHECK_MS = 5000;
const DEFAULT_PREFS = { fontSize: 13, opacity: 1, alwaysOnTop: true, autoStart: true, translationPaused: false };
const STARTED_HIDDEN = process.argv.includes("--hidden");

// Config -----------------------------------------------------------------

const configPath = () => path.join(app.getPath("userData"), "config.json");

function loadConfig() {
  try {
    return JSON.parse(fs.readFileSync(configPath(), "utf8"));
  } catch {
    return {};
  }
}

function saveConfig(patch) {
  config = { ...config, ...patch };
  fs.mkdirSync(path.dirname(configPath()), { recursive: true });
  fs.writeFileSync(configPath(), JSON.stringify(config, null, 2));
}

const prefs = () => ({ ...DEFAULT_PREFS, ...config.prefs });

let config = {};
let overlay = null;
let calibration = null;
let worker = null;
let restoreBounds = null; // bounds before "maximize"
let tray = null;
let watcher = null;
let quitting = false;
let userHidden = false; // hidden with ✕; panel detection must not re-show it
let panelPresent = null;

const send = (channel, payload) => overlay?.webContents.send(channel, payload);
const status = (text) => send("status", text);

// Pipeline: OCR lines -> new messages -> translation -> overlay -----------

const google = createGoogleTranslator();

// Translator settings ------------------------------------------------------
// The API key is stored encrypted with Windows DPAPI (safeStorage) and never sent
// to the renderer; the UI only learns whether a key is saved.

const PROVIDERS = {
  google: { label: "Google", baseUrl: "" },
  openrouter: { label: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1" },
  lmstudio: { label: "LM Studio", baseUrl: "http://localhost:1234/v1" },
  anthropic: { label: "Anthropic", baseUrl: "https://api.anthropic.com" },
  claudecode: { label: "Claude Code", baseUrl: "" },
  custom: { label: "Personalizado", baseUrl: "" },
};

const translatorSettings = () => ({ provider: "google", baseUrl: "", model: "", ...config.translator });

// Anthropic always uses its own endpoint, whatever URL the form holds.
const effectiveUrl = (provider, baseUrl) =>
  provider === "anthropic" ? PROVIDERS.anthropic.baseUrl : baseUrl || PROVIDERS[provider]?.baseUrl || "";

// The saved key is bound to the provider + server it was saved for and is only
// returned for that same pair, so it can never be sent to a different server.
function readApiKey(provider, baseUrl) {
  const t = config.translator;
  const enc = t?.apiKeyEnc;
  if (!enc || !safeStorage.isEncryptionAvailable()) return "";
  if (t.keyBinding !== keyBinding(provider, effectiveUrl(provider, baseUrl))) return "";
  try {
    return safeStorage.decryptString(Buffer.from(enc, "base64"));
  } catch {
    return "";
  }
}

// The Claude Code provider runs the user's own installed `claude` CLI. Found with
// `where`, falling back to the default install location of the native installer.
let claudeExe;
function findClaudeExecutable() {
  if (claudeExe !== undefined) return claudeExe;
  claudeExe = null;
  try {
    const found = execFileSync("where", ["claude"], { encoding: "utf8", windowsHide: true })
      .split(/\r?\n/)
      .map((l) => l.trim())
      .find((l) => /\.(exe|cmd)$/i.test(l));
    if (found) claudeExe = found;
  } catch {
    // not on PATH
  }
  const fallback = path.join(app.getPath("home"), ".local", "bin", "claude.exe");
  if (!claudeExe && fs.existsSync(fallback)) claudeExe = fallback;
  return claudeExe;
}

function llmFor(settings, apiKey) {
  if (settings.provider === "anthropic") {
    if (!apiKey) throw new Error("Falta la clave API de Anthropic");
    return createAnthropicTranslator({ apiKey, model: settings.model || ANTHROPIC_DEFAULT_MODEL });
  }
  if (settings.provider === "claudecode") {
    const executablePath = findClaudeExecutable();
    if (!executablePath) {
      throw new Error("Claude Code no está instalado en este PC (instálalo y ejecuta `claude auth login`)");
    }
    return createClaudeCodeTranslator({ model: settings.model || CLAUDE_CODE_MODELS[0], executablePath });
  }
  const baseUrl = effectiveUrl(settings.provider, settings.baseUrl);
  if (!baseUrl) throw new Error("Falta la URL del servidor");
  return createOpenAICompatibleTranslator({ baseUrl, apiKey, model: settings.model });
}

function buildTranslator() {
  const s = translatorSettings();
  if (s.provider === "google" || !s.model) {
    translatorIssue = "";
    return googleOnly();
  }
  const label = PROVIDERS[s.provider]?.label ?? "LLM";
  let llm;
  try {
    llm = llmFor(s, readApiKey(s.provider, s.baseUrl));
  } catch (err) {
    // Never let a bad translator setting (missing or unreadable key...) stop the
    // app from starting: translate with Google and say why.
    translatorIssue = `${label}: ${err.message}. Se usa Google.`;
    return googleOnly();
  }
  translatorIssue = "";
  return createCachedTranslator(
    createFallbackTranslator({ name: label, translator: llm }, { name: "Google", translator: google }),
  );
}

let translatorIssue = "";
function googleOnly() {
  return createCachedTranslator({ translate: async (t, l) => ({ ...(await google.translate(t, l)), provider: "Google" }) });
}

let translator = null;
const providerLabel = () => {
  const s = translatorSettings();
  return s.provider === "google" || !s.model ? "Google" : `${PROVIDERS[s.provider]?.label ?? "LLM"} · ${s.model}`;
};

// Diagnostic log (userData/debug.log): OCR reads and what the overlay received, so a
// "stuck" report can be checked against evidence. Small: rotated at 2 MB.
// WCT_DEBUG=1 also logs every capture tick.
const DEBUG_MAX_BYTES = 2 * 1024 * 1024;
function debug(...parts) {
  try {
    const file = path.join(app.getPath("userData"), "debug.log");
    if (fs.existsSync(file) && fs.statSync(file).size > DEBUG_MAX_BYTES) fs.renameSync(file, `${file}.old`);
    const line = `${new Date().toISOString()} ${parts.map((p) => (typeof p === "string" ? p : JSON.stringify(p))).join(" ")}\n`;
    fs.appendFileSync(file, line);
  } catch {
    // logging must never break the app
  }
}

// Paused translation keeps new messages untranslated; any of them can be translated
// later by clicking it in the overlay. Text from the pixel strip is exact, so no OCR
// corrections are applied.
const pipeline = createMessagePipeline({
  getTranslator: () => translator,
  send,
  isPaused: () => prefs().translationPaused,
  ocrFixes: false,
});

// Strip reader ------------------------------------------------------------
// The addon draws each chat message as a small grid of colored cells at the top-left
// of the WoW window; strip-worker.ps1 captures that corner and this decodes it.

function stopWorker() {
  if (worker) {
    const proc = worker;
    worker = null;
    proc.kill();
  }
}

function startWorker() {
  stopWorker();
  const proc = spawn("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", STRIP_WORKER], {
    windowsHide: true,
  });
  worker = proc;
  const layout = {}; // cell size and offset found last time
  let lastSeq = null;
  send("ocr:state", "starting");
  createInterface({ input: proc.stdout }).on("line", (line) => {
    let data;
    try {
      data = JSON.parse(line);
    } catch {
      return;
    }
    if (data.error) {
      debug("worker error", data.error);
      send("ocr:state", "error");
      return status(`Lector: ${data.error}`);
    }
    if (data.wow === false) return send("ocr:state", "idle");
    if (data.strip === false) {
      debug("strip absent");
      send("ocr:state", "nostrip");
      return onPanelPresence(false);
    }
    if (!data.patch) return;
    const frame = findStrip(Buffer.from(data.patch, "base64"), data.w, data.h, layout);
    if (!frame) return debug("strip undecodable");
    if (panelPresent !== true) onPanelPresence(true);
    send("ocr:state", "ok");
    status("");
    if (frame.seq === lastSeq) return;
    lastSeq = frame.seq;
    if (!frame.message) return; // idle frame: addon running, nothing new
    debug("strip", { seq: frame.seq, cell: layout.cell, channel: frame.message.channel });
    pipeline.addMessages([frame.message]);
  });
  proc.stderr.on("data", (d) => console.error(`[strip] ${d}`));
  proc.on("exit", (code) => {
    if (worker === proc) {
      worker = null;
      send("ocr:state", "error");
      status(`El lector se detuvo (código ${code}).`);
    }
  });
}

// The addon panel disappeared (addon disabled, /wct, Alt+Z, loading screen):
// hide and wait. It came back: show again unless the user hid the window.
function onPanelPresence(present) {
  panelPresent = present;
  if (present) {
    if (!userHidden) revealOverlay();
  } else {
    overlay?.hide();
  }
  updateTray();
}

// Show without stealing focus and force a repaint (a transparent window may keep
// showing its last frame after hide/show until something moves it).
function revealOverlay() {
  if (!overlay) return;
  overlay.showInactive();
  overlay.webContents.invalidate();
}

// Windows -----------------------------------------------------------------

function createOverlay() {
  const b = config.overlayBounds ?? {};
  const wa = screen.getPrimaryDisplay().workArea;
  overlay = new BrowserWindow({
    width: b.width ?? 560,
    height: b.height ?? 420,
    x: b.x ?? wa.x + wa.width - 580,
    y: b.y ?? wa.y + 20,
    minWidth: 320,
    minHeight: 180,
    frame: false,
    // Opaque on purpose: Windows can stop repainting a transparent window that sits
    // over a game until it is moved (messages arrived but were not drawn).
    transparent: false,
    backgroundColor: "#0e1016",
    resizable: true,
    show: false,
    title: "WoW Chat Translator",
    // Keep rendering while hidden: otherwise the window can reappear showing stale
    // content until it is moved (seen after a /reload loading screen).
    webPreferences: { preload: path.join(here, "preload.cjs"), backgroundThrottling: false },
  });
  applyAlwaysOnTop();
  // Invisible to screen capture, so the overlay can sit on top of the addon panel
  // and the OCR still reads the panel underneath (verified on this machine).
  overlay.setContentProtection(true);
  overlay.loadFile(path.join(here, "renderer", "overlay.html"));
  overlay.once("ready-to-show", () => {
    // Started by hand: show the window even before WoW opens. Started with
    // Windows (--hidden): stay in the tray until the game appears.
    if (!STARTED_HIDDEN) overlay.showInactive();
    if (!watcher?.running) status("Esperando a que abras WoW…");
  });
  const persist = () => {
    if (!restoreBounds) saveConfig({ overlayBounds: overlay.getBounds() });
  };
  overlay.on("moved", persist);
  overlay.on("resized", persist);
  // The ✕ button and Alt+F4 only hide; quitting is done from the tray menu.
  overlay.on("close", (e) => {
    if (!quitting) {
      e.preventDefault();
      overlay.hide();
    }
  });
}

function applyAlwaysOnTop() {
  overlay?.setAlwaysOnTop(prefs().alwaysOnTop, "screen-saver");
}

function toggleMaximize() {
  if (!overlay) return;
  if (restoreBounds) {
    overlay.setBounds(restoreBounds);
    restoreBounds = null;
  } else {
    // Native maximize is unreliable for transparent frameless windows on Windows.
    restoreBounds = overlay.getBounds();
    overlay.setBounds(screen.getDisplayMatching(restoreBounds).workArea);
  }
  send("win:maximized", Boolean(restoreBounds));
}

// Exact panel bounds (physical px), detected from the screen around the calibrated
// region. The overlay is excluded from capture, so this works even while docked.
function detectPanel(r) {
  return new Promise((resolve) => {
    execFile(
      "powershell.exe",
      ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", DETECT_PANEL,
        "-X", String(r.x), "-Y", String(r.y), "-W", String(r.w), "-H", String(r.h)],
      { windowsHide: true },
      (err, stdout) => {
        try {
          const b = JSON.parse(stdout);
          resolve(!err && Number.isFinite(b.w) && b.w >= r.w && b.h >= r.h ? b : r);
        } catch {
          resolve(r);
        }
      },
    );
  });
}

async function dockOverPanel() {
  const r = config.region;
  if (!overlay || !r) return status("Primero calibra la zona del panel");
  const p = await detectPanel(r);
  const display = screen.getDisplayNearestPoint({ x: p.x, y: p.y });
  const sf = display.scaleFactor;
  if (restoreBounds) {
    restoreBounds = null;
    send("win:maximized", false);
  }
  // Cover the panel exactly (rounded outwards). If the panel is smaller than the
  // window's minimum size, grow upwards/rightwards from its bottom-left corner.
  const x = Math.floor(p.x / sf);
  const bottom = Math.ceil((p.y + p.h) / sf);
  const width = Math.max(320, Math.ceil((p.x + p.w) / sf) - x);
  const height = Math.max(180, bottom - Math.floor(p.y / sf));
  overlay.setBounds({ x, y: bottom - height, width, height });
  saveConfig({ overlayBounds: overlay.getBounds() });
}

function openCalibration() {
  if (calibration) return;
  stopWorker();
  overlay?.hide(); // it may be covering the panel the user needs to select
  settingsWin?.close(); // always on top too: it would sit over the selection screen
  const display = screen.getDisplayNearestPoint(screen.getCursorScreenPoint());
  calibration = new BrowserWindow({
    ...display.bounds,
    frame: false,
    transparent: true,
    alwaysOnTop: true,
    skipTaskbar: true,
    resizable: false,
    movable: false,
    webPreferences: { preload: path.join(here, "preload.cjs") },
  });
  calibration.setAlwaysOnTop(true, "screen-saver");
  calibration.loadFile(path.join(here, "renderer", "calibrate.html"));
  calibration.on("closed", () => {
    calibration = null;
    ipcMain.removeAllListeners("calib:done");
    revealOverlay();
    if (watcher?.running) startWorker();
  });

  ipcMain.once("calib:done", (_e, rect) => {
    const sf = display.scaleFactor;
    saveConfig({
      region: {
        x: Math.round((display.bounds.x + rect.x) * sf),
        y: Math.round((display.bounds.y + rect.y) * sf),
        w: Math.max(20, Math.round(rect.w * sf)),
        h: Math.max(20, Math.round(rect.h * sf)),
      },
    });
    calibration?.close();
  });
}

function focusCompose() {
  if (!overlay) return;
  overlay.show();
  overlay.focus();
  send("compose:focus");
}

// Game watcher: show with WoW, hide when it closes ------------------------

function isGameRunning() {
  return new Promise((resolve, reject) => {
    execFile(
      "tasklist",
      ["/FI", `IMAGENAME eq ${GAME_EXE}`, "/NH", "/FO", "CSV"],
      { windowsHide: true },
      (err, stdout) => (err ? reject(err) : resolve(stdout.toLowerCase().includes(`"${GAME_EXE.toLowerCase()}"`))),
    );
  });
}

// The only global hotkey (quick compose). It exists only while the game runs, so it
// never steals the combination from other programs. Calibration and the
// translated/original toggle have buttons and no hotkey: Ctrl+Shift+R/T clashed
// with the user's in-game keybinds.
function registerShortcuts() {
  globalShortcut.register("CommandOrControl+Shift+Enter", () => focusCompose());
}

function onGameStart() {
  userHidden = false;
  registerShortcuts();
  // With a calibrated region, panel detection decides when to show the window.
  startWorker();
  updateTray();
}

function onGameExit() {
  globalShortcut.unregisterAll();
  stopWorker();
  overlay?.hide();
  panelPresent = null;
  status("Esperando a que abras WoW…");
  send("ocr:state", "idle");
  updateTray();
}

function startGameWatcher() {
  watcher = createGameWatcher({ isRunning: isGameRunning, onStart: onGameStart, onExit: onGameExit });
  watcher.check();
  setInterval(() => watcher.check(), GAME_CHECK_MS);
}

// Tray + start with Windows -------------------------------------------------

function applyAutoStart() {
  app.setLoginItemSettings({
    name: "WoW Chat Translator", // registry value name shown in Task Manager > Startup
    openAtLogin: prefs().autoStart,
    path: process.execPath,
    // In development process.execPath is electron.exe, so pass the app folder too.
    args: app.isPackaged ? ["--hidden"] : [app.getAppPath(), "--hidden"],
  });
}

function showOverlay() {
  userHidden = false;
  overlay?.show();
  overlay?.webContents.invalidate();
}

function updateTray() {
  if (!tray) return;
  const p = prefs();
  const state = !watcher?.running
    ? "esperando a WoW"
    : panelPresent === false
      ? "panel del addon no visible, en espera"
      : "WoW abierto";
  tray.setToolTip(`WoW Chat Translator — ${state}`);
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: "Mostrar ventana", click: showOverlay },
      { type: "separator" },
      {
        label: "Iniciar con Windows",
        type: "checkbox",
        checked: p.autoStart,
        click: (item) => {
          saveConfig({ prefs: { ...prefs(), autoStart: item.checked } });
          applyAutoStart();
        },
      },
      { type: "separator" },
      { label: "Salir", click: () => { quitting = true; app.quit(); } },
    ]),
  );
}

function createTray() {
  const icon = nativeImage.createFromPath(path.join(here, "..", "assets", "tray-32.png")).resize({ width: 16, height: 16 });
  tray = new Tray(icon);
  tray.on("click", showOverlay);
  updateTray();
}

// IPC ---------------------------------------------------------------------

ipcMain.on("calib:cancel", () => calibration?.close());
ipcMain.on("app:calibrate", () => openCalibration());
ipcMain.on("app:close", () => {
  userHidden = true;
  overlay?.hide();
});
ipcMain.on("win:minimize", () => overlay?.minimize());
ipcMain.on("win:maximize", () => toggleMaximize());
ipcMain.on("win:dock", () => dockOverPanel());
// Transparent frameless windows cannot be resized natively on Windows; the renderer
// drives a custom grip instead.
ipcMain.on("win:resize", (_e, { width, height }) => {
  restoreBounds = null;
  overlay?.setSize(Math.max(320, Math.round(width)), Math.max(180, Math.round(height)));
});
ipcMain.on("win:resize-end", () => overlay && saveConfig({ overlayBounds: overlay.getBounds() }));
ipcMain.on("win:blur", () => overlay?.blur());
// Bring WoW back to the front after the compose box (no keystrokes are sent). The
// helper is started while this app has the focus, which is what Windows requires.
ipcMain.on("win:focus-game", () => {
  execFile("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", FOCUS_WOW], { windowsHide: true }, (err, stdout) => {
    let ok = false;
    try {
      ok = JSON.parse(stdout).ok === true;
    } catch {
      // ignore
    }
    debug("focus-game", { ok, error: err?.message, out: String(stdout).trim() });
    if (!ok) overlay?.blur();
  });
});

ipcMain.handle("translator:get", () => {
  const s = translatorSettings();
  const hasKey = Boolean(s.apiKeyEnc) && s.keyBinding === keyBinding(s.provider, effectiveUrl(s.provider, s.baseUrl));
  return { provider: s.provider, baseUrl: s.baseUrl, model: s.model, hasKey, label: providerLabel() };
});

// apiKey: undefined keeps the saved key (only if provider and server are unchanged),
// "" deletes it, a string replaces it.
ipcMain.handle("translator:set", (_e, { provider, baseUrl, model, apiKey }) => {
  if (!PROVIDERS[provider]) return { ok: false, error: "Proveedor desconocido" };
  const next = { provider, baseUrl: String(baseUrl ?? "").trim(), model: String(model ?? "").trim() };
  const binding = keyBinding(provider, effectiveUrl(provider, next.baseUrl));
  const prev = config.translator;
  if (apiKey === undefined) {
    if (prev?.apiKeyEnc && prev.keyBinding === binding) {
      next.apiKeyEnc = prev.apiKeyEnc;
      next.keyBinding = binding;
    }
  } else if (apiKey) {
    if (!safeStorage.isEncryptionAvailable()) return { ok: false, error: "Windows no permite cifrar la clave; no se guardó" };
    next.apiKeyEnc = safeStorage.encryptString(String(apiKey).trim()).toString("base64");
    next.keyBinding = binding;
  }
  if (provider === "anthropic" && !next.apiKeyEnc) {
    return { ok: false, error: "Pega tu clave API de Anthropic antes de guardar" };
  }
  saveConfig({ translator: next });
  translator = buildTranslator();
  if (translatorIssue) status(translatorIssue);
  send("translator:changed", providerLabel());
  return { ok: true, label: providerLabel() };
});

ipcMain.handle("translator:models", async (_e, { provider, baseUrl, apiKey }) => {
  try {
    const key = apiKey === undefined ? readApiKey(provider, baseUrl) : apiKey;
    return { ok: true, models: await llmFor({ provider, baseUrl }, key).listModels() };
  } catch (err) {
    return { ok: false, error: err.message };
  }
});

ipcMain.handle("translator:test", async (_e, { provider, baseUrl, model, apiKey }) => {
  const sample = "LFM ZG need heal, pst";
  try {
    if (provider === "google") return { ok: true, sample, ...(await google.translate(sample, TARGET_LANG)) };
    if (!model) return { ok: false, error: "Elige un modelo" };
    const key = apiKey === undefined ? readApiKey(provider, baseUrl) : apiKey;
    return { ok: true, sample, ...(await llmFor({ provider, baseUrl, model }, key).translate(sample, TARGET_LANG)) };
  } catch (err) {
    return { ok: false, error: err.message };
  }
});

ipcMain.handle("prefs:get", () => prefs());
ipcMain.handle("prefs:set", (_e, patch) => {
  saveConfig({ prefs: { ...prefs(), ...patch } });
  if ("alwaysOnTop" in patch) applyAlwaysOnTop();
  if ("autoStart" in patch) {
    applyAutoStart();
    updateTray();
  }
  // Both windows show preferences: keep them in sync.
  send("prefs:changed", prefs());
  settingsWin?.webContents.send("prefs:changed", prefs());
  return prefs();
});

// Settings window ---------------------------------------------------------
// A separate window so the options never get squeezed into a small overlay.

let settingsWin = null;

function openSettings() {
  if (settingsWin) {
    settingsWin.show();
    settingsWin.focus();
    return;
  }
  const area = screen.getDisplayMatching(overlay?.getBounds() ?? screen.getPrimaryDisplay().bounds).workArea;
  const width = 500;
  const height = 600;
  settingsWin = new BrowserWindow({
    width,
    height,
    x: Math.round(area.x + (area.width - width) / 2),
    y: Math.round(area.y + (area.height - height) / 2),
    minWidth: 420,
    minHeight: 420,
    frame: false,
    resizable: true,
    show: false,
    backgroundColor: "#0f141d",
    title: "Ajustes · WoW Chat Translator",
    webPreferences: { preload: path.join(here, "preload.cjs") },
  });
  settingsWin.setAlwaysOnTop(true, "screen-saver"); // above the game, like the overlay
  settingsWin.setContentProtection(true); // never read by the OCR if placed over the panel
  settingsWin.loadFile(path.join(here, "renderer", "settings.html"));
  settingsWin.once("ready-to-show", () => settingsWin.show());
  settingsWin.on("closed", () => {
    settingsWin = null;
  });
}

ipcMain.on("settings:open", () => openSettings());
ipcMain.on("settings:close", () => settingsWin?.close());
ipcMain.on("chat:clear", () => send("chat:clear"));
ipcMain.on("chat:translate", (_e, id) => pipeline.translateById(Number(id)));
ipcMain.on("debug:ui", (_e, info) => debug("ui", info));
ipcMain.on("app:quit", () => {
  quitting = true;
  app.quit();
});

// Outgoing message: translate (or pass through) and copy to the clipboard.
// The app never types into the game; the user pastes with Ctrl+V.
ipcMain.handle("compose:translate", async (_e, { text, lang }) => {
  const clean = String(text ?? "").trim();
  if (!clean) return { ok: false, error: "Mensaje vacío" };
  try {
    const out = lang === "es" ? clean : (await translator.translate(clean, lang)).text;
    clipboard.writeText(out);
    return { ok: true, text: out };
  } catch (err) {
    return { ok: false, error: err.message };
  }
});

// A second launch (e.g. the .bat while it already runs) just shows the window.
if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on("second-instance", showOverlay);
  app.whenReady().then(() => {
    config = loadConfig();
    translator = buildTranslator();
    createOverlay();
    createTray();
    applyAutoStart();
    startGameWatcher();
  });
}

app.on("before-quit", () => {
  quitting = true;
});
// Keep running in the tray when the window is hidden.
app.on("window-all-closed", () => {});

app.on("will-quit", () => {
  globalShortcut.unregisterAll();
  stopWorker();
});
