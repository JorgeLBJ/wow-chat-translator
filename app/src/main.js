import { app, BrowserWindow, clipboard, globalShortcut, ipcMain, screen } from "electron";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseLines } from "./core/parser.js";
import { createTracker } from "./core/tracker.js";
import { looksSpanish } from "./core/language.js";
import { channelTab } from "./core/channels.js";
import { createGoogleTranslator } from "./core/translators/google.js";
import { createCachedTranslator } from "./core/cache.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const WORKER = path.join(here, "..", "ocr", "ocr-worker.ps1");
const TARGET_LANG = "es";
const DEFAULT_PREFS = { fontSize: 13, opacity: 0.94, alwaysOnTop: true };

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

const send = (channel, payload) => overlay?.webContents.send(channel, payload);
const status = (text) => send("status", text);

// Pipeline: OCR lines -> new messages -> translation -> overlay -----------

const tracker = createTracker();
const translator = createCachedTranslator(createGoogleTranslator());
let nextId = 1;

function handleOcrLines(lines) {
  const fresh = tracker.newMessages(parseLines(lines));
  for (const msg of fresh) {
    const item = {
      id: nextId++,
      time: Date.now(),
      channel: msg.channel,
      tab: channelTab(msg.channel),
      cls: msg.cls,
      player: msg.player,
      original: msg.text,
      translated: null,
      lang: null,
      status: "pending",
    };
    if (looksSpanish(msg.text)) {
      send("chat:add", { ...item, lang: "es", status: "done" });
      continue;
    }
    send("chat:add", item);
    translator
      .translate(msg.text, TARGET_LANG)
      .then((r) => {
        const same = r.source === TARGET_LANG || r.text.trim().toLowerCase() === msg.text.trim().toLowerCase();
        send("chat:update", { id: item.id, lang: r.source, translated: same ? null : r.text, status: "done" });
      })
      .catch((err) => send("chat:update", { id: item.id, status: "error", error: err.message }));
  }
}

// OCR worker --------------------------------------------------------------

function stopWorker() {
  if (worker) {
    const proc = worker;
    worker = null;
    proc.kill();
  }
}

function startWorker() {
  stopWorker();
  const r = config.region;
  if (!r) {
    status("Calibra la zona del panel desde ⚙ o con Ctrl+Shift+R");
    return;
  }
  const args = [
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", WORKER,
    "-X", String(r.x), "-Y", String(r.y), "-W", String(r.w), "-H", String(r.h),
    "-Scale", String(config.ocrScale ?? 2),
  ];
  const proc = spawn("powershell.exe", args, { windowsHide: true });
  worker = proc;
  send("ocr:state", "starting");
  createInterface({ input: proc.stdout }).on("line", (line) => {
    let data;
    try {
      data = JSON.parse(line);
    } catch {
      return;
    }
    if (data.error) {
      send("ocr:state", "error");
      return status(`OCR: ${data.error}`);
    }
    const lines = Array.isArray(data.lines) ? data.lines : data.lines ? [data.lines] : [];
    send("ocr:state", "ok");
    status("");
    handleOcrLines(lines);
  });
  proc.stderr.on("data", (d) => console.error(`[ocr] ${d}`));
  proc.on("exit", (code) => {
    if (worker === proc) {
      worker = null;
      send("ocr:state", "error");
      status(`El lector OCR se detuvo (código ${code}). Recalibra para reiniciarlo.`);
    }
  });
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
    transparent: true,
    resizable: true,
    show: false,
    title: "WoW Chat Translator",
    webPreferences: { preload: path.join(here, "preload.cjs") },
  });
  applyAlwaysOnTop();
  // Invisible to screen capture, so the overlay can sit on top of the addon panel
  // and the OCR still reads the panel underneath (verified on this machine).
  overlay.setContentProtection(true);
  overlay.loadFile(path.join(here, "renderer", "overlay.html"));
  overlay.once("ready-to-show", () => {
    overlay.showInactive();
    startWorker();
  });
  const persist = () => {
    if (!restoreBounds) saveConfig({ overlayBounds: overlay.getBounds() });
  };
  overlay.on("moved", persist);
  overlay.on("resized", persist);
  overlay.on("closed", () => app.quit());
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

function dockOverPanel() {
  const r = config.region;
  if (!overlay || !r) return status("Primero calibra la zona del panel");
  const display = screen.getDisplayNearestPoint({ x: r.x, y: r.y });
  const sf = display.scaleFactor;
  restoreBounds = null;
  send("win:maximized", false);
  overlay.setBounds({
    x: Math.round(r.x / sf),
    y: Math.round(r.y / sf),
    width: Math.max(320, Math.round(r.w / sf)),
    height: Math.max(180, Math.round(r.h / sf)),
  });
  saveConfig({ overlayBounds: overlay.getBounds() });
}

function openCalibration() {
  if (calibration) return;
  stopWorker();
  overlay?.hide(); // it may be covering the panel the user needs to select
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
    overlay?.showInactive();
    startWorker();
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

// IPC ---------------------------------------------------------------------

ipcMain.on("calib:cancel", () => calibration?.close());
ipcMain.on("app:calibrate", () => openCalibration());
ipcMain.on("app:close", () => app.quit());
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

ipcMain.handle("prefs:get", () => prefs());
ipcMain.handle("prefs:set", (_e, patch) => {
  saveConfig({ prefs: { ...prefs(), ...patch } });
  if ("alwaysOnTop" in patch) applyAlwaysOnTop();
  return prefs();
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

app.whenReady().then(() => {
  config = loadConfig();
  createOverlay();
  globalShortcut.register("CommandOrControl+Shift+T", () => send("view:toggle"));
  globalShortcut.register("CommandOrControl+Shift+R", () => openCalibration());
  globalShortcut.register("CommandOrControl+Shift+Enter", () => focusCompose());
  if (!config.region) overlay.once("ready-to-show", () => openCalibration());
});

app.on("will-quit", () => {
  globalShortcut.unregisterAll();
  stopWorker();
});
