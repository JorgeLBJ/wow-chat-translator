// Settings window: translator, window and general options. It talks to the main
// process only; the main process forwards changes to the chat overlay.

const $ = (id) => document.getElementById(id);

const PRESET_URLS = { openrouter: "https://openrouter.ai/api/v1", lmstudio: "http://localhost:1234/v1", custom: "" };
const DEFAULT_MODELS = { anthropic: "claude-haiku-5-5", claudecode: "claude-haiku-5-5" };
const KEY_HINTS = {
  openrouter: "Créala en openrouter.ai → Keys.",
  lmstudio: "No hace falta para LM Studio. Abre LM Studio → Developer → Start Server y carga un modelo.",
  custom: "Solo si tu servidor la pide.",
  anthropic:
    "Créala en console.anthropic.com → API Keys. Si esa organización está vinculada a tu plan de Claude, usa sus créditos mensuales.",
  claudecode:
    "Sin clave: usa tu sesión de Claude Code (claude auth login) y el cupo de tu plan, el mismo que usas para programar. Tarda unos 3-4 s por mensaje.",
};
const HIDE_URL = new Set(["anthropic", "claudecode"]);
const HIDE_KEY = new Set(["claudecode"]);

let prefs = { fontSize: 13, opacity: 1, alwaysOnTop: true, autoStart: true };
let savedTranslator = null;
let keyTouched = false;

// Tabs -------------------------------------------------------------------------

function showPane(id) {
  document.querySelectorAll(".stab").forEach((b) => b.classList.toggle("on", b.dataset.pane === id));
  document.querySelectorAll(".pane").forEach((p) => { p.hidden = p.id !== id; });
}
document.querySelectorAll(".stab").forEach((b) => { b.onclick = () => showPane(b.dataset.pane); });

$("s-close").onclick = () => window.wct.closeSettings();
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") window.wct.closeSettings();
});

// Translator --------------------------------------------------------------------

const currentProvider = () => $("t-provider").value;

function formSettings() {
  return {
    provider: currentProvider(),
    baseUrl: $("t-url").value.trim(),
    model: $("t-model").value.trim(),
    // undefined = keep the saved key (it is never sent back to the UI)
    apiKey: keyTouched ? $("t-key").value.trim() : undefined,
  };
}

function showResult(text, kind) {
  $("t-result").className = kind ?? "";
  $("t-result").textContent = text;
}

function syncProvider() {
  const p = currentProvider();
  $("t-llm").hidden = p === "google";
  $("t-url-row").hidden = HIDE_URL.has(p);
  $("t-key-row").hidden = HIDE_KEY.has(p);
  const sameProvider = savedTranslator && savedTranslator.provider === p;
  $("t-url").value = sameProvider && savedTranslator.baseUrl ? savedTranslator.baseUrl : PRESET_URLS[p] ?? "";
  $("t-model").value = sameProvider && savedTranslator.model ? savedTranslator.model : DEFAULT_MODELS[p] ?? "";
  keyTouched = false;
  $("t-key").value = "";
  $("t-key").placeholder = sameProvider && savedTranslator.hasKey ? "•••••• guardada (escribe para cambiarla)" : "sin clave";
  $("t-key-hint").textContent = KEY_HINTS[p] ?? "";
  $("t-models").replaceChildren();
  showResult("");
}

$("t-provider").onchange = syncProvider;
$("t-key").oninput = () => { keyTouched = true; };

$("t-load").onclick = async () => {
  showResult("Cargando modelos…");
  const res = await window.wct.listModels(formSettings());
  if (!res.ok) return showResult(`No se pudo cargar la lista: ${res.error}`, "err");
  $("t-models").replaceChildren(...res.models.map((id) => {
    const o = document.createElement("option");
    o.value = id;
    return o;
  }));
  if (!$("t-model").value && res.models.length === 1) $("t-model").value = res.models[0];
  showResult(`${res.models.length} modelos disponibles. Haz clic en el campo Modelo para elegir.`, "ok");
};

$("t-test").onclick = async () => {
  showResult("Probando…");
  const res = await window.wct.testTranslator(formSettings());
  if (!res.ok) return showResult(`Error: ${res.error}`, "err");
  showResult(`«${res.sample}» → «${res.text}»`, "ok");
};

$("t-save").onclick = async () => {
  const s = formSettings();
  if (s.provider !== "google" && !s.model) return showResult("Elige un modelo antes de guardar.", "err");
  const res = await window.wct.setTranslator(s);
  if (!res.ok) return showResult(`Error: ${res.error}`, "err");
  savedTranslator = await window.wct.getTranslator();
  showResult(`Guardado. Traduciendo con: ${res.label}`, "ok");
};

// Window + general ----------------------------------------------------------------

function applyPrefs() {
  const pct = Math.round(prefs.opacity * 100);
  $("w-opacity").value = pct;
  $("w-opacity-val").textContent = `${pct} %`;
  $("w-font").value = prefs.fontSize;
  $("w-font-val").textContent = `${prefs.fontSize} px`;
  $("w-ontop").checked = prefs.alwaysOnTop;
  $("g-autostart").checked = prefs.autoStart !== false;
}

const savePrefs = async (patch) => {
  prefs = await window.wct.setPrefs(patch);
  applyPrefs();
};

$("w-opacity").oninput = (e) => {
  $("w-opacity-val").textContent = `${e.target.value} %`;
  window.wct.setPrefs({ opacity: Number(e.target.value) / 100 });
};
$("w-font").oninput = (e) => {
  $("w-font-val").textContent = `${e.target.value} px`;
  window.wct.setPrefs({ fontSize: Number(e.target.value) });
};
$("w-ontop").onchange = (e) => savePrefs({ alwaysOnTop: e.target.checked });
$("w-calib").onclick = () => window.wct.calibrate();
$("w-dock").onclick = () => window.wct.dock();
$("g-autostart").onchange = (e) => savePrefs({ autoStart: e.target.checked });
$("g-clear").onclick = () => window.wct.clearMessages();
$("g-quit").onclick = () => window.wct.quit();

// Kept in sync when another window (the chat overlay) changes a preference.
window.wct.onPrefsChanged((p) => {
  prefs = p;
  applyPrefs();
});

// Init ------------------------------------------------------------------------------

Promise.all([window.wct.getTranslator(), window.wct.getPrefs()]).then(([t, p]) => {
  savedTranslator = t;
  $("t-provider").value = t.provider;
  syncProvider();
  prefs = p;
  applyPrefs();
});
