// Overlay UI. All chat text is inserted with textContent: it comes from other players.

const MAX_ITEMS = 300;
const $ = (id) => document.getElementById(id);

const CLASS_COLORS = {
  WARRIOR: "#C69B6D", PALADIN: "#F48CBA", HUNTER: "#AAD372", ROGUE: "#FFF468", PRIEST: "#FFFFFF",
  SHAMAN: "#0070DD", MAGE: "#3FC7EB", WARLOCK: "#8788EE", DRUID: "#FF7C0A", DEATHKNIGHT: "#C41E3A",
  MONK: "#00FF98", DEMONHUNTER: "#A330C9", EVOKER: "#33937F",
};
const TABS = [
  { key: "all", label: "Todos" },
  { key: "general", label: "General" },
  { key: "trade", label: "Comercio" },
  { key: "guild", label: "Hermandad" },
  { key: "group", label: "Grupo" },
  { key: "whisper", label: "Susurros" },
];
const TAB_COLORS = { general: "#c8a165", trade: "#d38b8b", guild: "#40c040", group: "#8f9cff", whisper: "#ff80ff" };
const CHANNEL_LABELS = {
  general: "General", trade: "Comercio", comercio: "Comercio", guild: "Hermandad", officer: "Oficial",
  party: "Grupo", raid: "Banda", instance: "Instancia", whisper: "Susurro", say: "Decir", yell: "Gritar",
};

const items = new Map();
const unread = Object.fromEntries(TABS.map((t) => [t.key, 0]));
let mode = "t";
let activeTab = "all";
let outLang = "en";
let prefs = { fontSize: 13, opacity: 1, alwaysOnTop: true };

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

const visible = (item) => activeTab === "all" || item.tab === activeTab;
const hhmm = (t) => new Date(t).toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" });

// Tabs ----------------------------------------------------------------------

function renderTabs() {
  const box = $("tabs");
  box.replaceChildren();
  for (const t of TABS) {
    const b = el("button", "tab" + (t.key === activeTab ? " on" : ""), t.label);
    if (unread[t.key] > 0 && t.key !== activeTab && t.key !== "all") b.append(el("span", "n", String(unread[t.key])));
    b.onclick = () => {
      activeTab = t.key;
      unread[t.key] = 0;
      renderTabs();
      renderAll();
    };
    box.append(b);
  }
}

// Messages ------------------------------------------------------------------

function render(item) {
  const node = item.node;
  node.replaceChildren();
  node.hidden = !visible(item);
  const showTranslated = mode === "t" && item.translated;
  node.className = "m" + (item.status === "pending" ? " pending" : "") + (showTranslated ? " has-orig" : "");

  const tabColor = TAB_COLORS[item.tab] ?? "#9aa3ad";
  const body = el("div", "body");
  const ch = el("span", "ch", CHANNEL_LABELS[item.channel.toLowerCase()] ?? item.channel);
  ch.style.color = tabColor;
  ch.style.background = `${tabColor}22`;
  const who = el("span", "who", item.player);
  who.style.color = CLASS_COLORS[item.cls] ?? "#d0d4da";
  body.append(ch, who);

  // Untranslated messages (translation paused, or it failed) are clickable: one
  // click translates just that message.
  const canTranslate = item.status === "paused" || item.status === "error";
  node.classList.toggle("can-translate", canTranslate);
  node.onclick = canTranslate
    ? () => {
        item.status = "pending";
        render(item);
        window.wct.translateMessage(item.id);
      }
    : null;
  if (canTranslate) node.title = "Clic para traducir este mensaje";
  else node.removeAttribute("title");

  if (item.status === "error") {
    const badge = el("span", "lg err", "error · traducir");
    badge.title = `${item.error ?? ""} (clic para reintentar)`;
    body.append(badge);
  } else if (item.status === "paused") {
    body.append(el("span", "lg paused", "⏸ traducir"));
  } else if (item.lang) {
    const badge = el("span", "lg", showTranslated ? `${item.lang.toUpperCase()}→ES` : item.lang.toUpperCase());
    if (item.provider) badge.title = item.fallbackReason ? `${item.provider} (LLM falló: ${item.fallbackReason})` : item.provider;
    body.append(badge);
  } else if (showTranslated) {
    body.append(el("span", "lg", "→ES"));
  }
  body.append(el("span", "txt", `: ${showTranslated ? item.translated : item.original}`));
  if (showTranslated) body.append(el("div", "orig", item.original));

  node.append(el("span", "ts", hhmm(item.time)), body);
}

function renderAll() {
  for (const item of items.values()) render(item);
  renderEmpty();
  $("list").scrollTop = $("list").scrollHeight;
}

function renderEmpty() {
  const list = $("list");
  let empty = list.querySelector(".empty");
  const any = [...items.values()].some(visible);
  if (any && empty) empty.remove();
  if (!any && !empty) {
    empty = el("div", "empty", "Sin mensajes todavía. Aparecerán aquí en cuanto alguien escriba en el chat.");
    list.prepend(empty);
  }
}

const nearBottom = () => {
  const l = $("list");
  return l.scrollHeight - l.scrollTop - l.clientHeight < 40;
};

window.wct.onAdd((data) => {
  const stick = nearBottom();
  const item = { ...data, node: el("div", "m") };
  items.set(item.id, item);
  $("list").append(item.node);
  render(item);
  if (item.tab && item.tab !== activeTab && activeTab !== "all") unread[item.tab]++;
  else if (item.tab && activeTab === "all") unread[item.tab] = 0;
  renderTabs();
  while (items.size > MAX_ITEMS) {
    const [id, old] = items.entries().next().value;
    old.node.remove();
    items.delete(id);
  }
  renderEmpty();
  if (stick) $("list").scrollTop = $("list").scrollHeight;
  const l = $("list");
  window.wct.debugUi?.({
    id: item.id,
    activeTab,
    tab: item.tab,
    shown: !item.node.hidden,
    visibility: document.visibilityState,
    stuckToBottom: stick,
    scroll: [Math.round(l.scrollTop), l.scrollHeight, l.clientHeight],
  });
});

window.wct.onUpdate((data) => {
  const item = items.get(data.id);
  if (!item) return;
  const stick = nearBottom();
  Object.assign(item, data);
  render(item);
  if (stick) $("list").scrollTop = $("list").scrollHeight;
});

function setMode(next) {
  mode = next;
  $("btn-t").classList.toggle("on", mode === "t");
  $("btn-o").classList.toggle("on", mode === "o");
  renderAll();
}

// Title bar -------------------------------------------------------------------

$("btn-t").onclick = () => setMode("t");
$("btn-o").onclick = () => setMode("o");
$("dock").onclick = () => window.wct.dock();
$("min").onclick = () => window.wct.minimize();
$("max").onclick = () => window.wct.maximize();
$("close").onclick = () => window.wct.close();
$("pin").onclick = () => savePrefs({ alwaysOnTop: !prefs.alwaysOnTop });
$("pause").onclick = () => savePrefs({ translationPaused: !prefs.translationPaused });
window.wct.onMaximized((maxed) => {
  document.body.classList.toggle("maxed", maxed);
  $("max").textContent = maxed ? "❐" : "☐";
  $("max").dataset.tip = maxed ? "Restaurar" : "Maximizar";
});

// Footer ----------------------------------------------------------------------

const OCR_LABELS = { ok: "OCR activo", starting: "OCR iniciando", error: "OCR detenido", idle: "Esperando a WoW" };
window.wct.onOcrState((state) => {
  $("dot").className = `dot ${state}`;
  $("ocr-label").textContent = OCR_LABELS[state] ?? "OCR";
  $("sub").textContent = { ok: "● leyendo", error: "● detenido", idle: "esperando a WoW" }[state] ?? "iniciando…";
});
window.wct.onStatus((text) => { $("status").textContent = text; });

function applyPrefs() {
  $("win").style.opacity = prefs.opacity;
  $("opacity").value = Math.round(prefs.opacity * 100);
  $("list").style.setProperty("--fs", `${prefs.fontSize}px`);
  $("pin").classList.toggle("on", prefs.alwaysOnTop);
  $("pin").dataset.tip = prefs.alwaysOnTop ? "Siempre encima: activado" : "Siempre encima: desactivado";
  const paused = Boolean(prefs.translationPaused);
  $("pause").classList.toggle("on", paused);
  $("pause").textContent = paused ? "▶" : "⏸";
  $("pause").dataset.tip = paused ? "Reanudar traducción" : "Pausar traducción";
  $("paused-note").textContent = paused ? "⏸ En pausa · clic en un mensaje para traducir" : "";
}

const savePrefs = async (patch) => {
  prefs = await window.wct.setPrefs(patch);
  applyPrefs();
};

$("opacity").oninput = (e) => {
  prefs.opacity = Number(e.target.value) / 100;
  applyPrefs();
};
$("opacity").onchange = () => window.wct.setPrefs({ opacity: prefs.opacity });

// Settings live in their own window; it reports changes back here.
$("gear").onclick = () => window.wct.openSettings();
window.wct.onPrefsChanged((p) => {
  prefs = p;
  applyPrefs();
});
window.wct.onClearMessages(() => {
  for (const item of items.values()) item.node.remove();
  items.clear();
  renderEmpty();
});

// Compose ---------------------------------------------------------------------

const input = $("msg");
const preview = $("preview");

document.querySelectorAll("#out-lang button").forEach((b) => {
  b.onclick = () => {
    outLang = b.dataset.lang;
    document.querySelectorAll("#out-lang button").forEach((x) => x.classList.toggle("on", x === b));
    input.focus();
  };
});

function showPreview(text, kind) {
  preview.className = `preview ${kind ?? ""}`;
  preview.textContent = text;
}

input.addEventListener("keydown", async (e) => {
  if (e.key === "Escape") {
    input.blur();
    window.wct.blur();
    return;
  }
  if (e.key !== "Enter") return;
  e.preventDefault();
  const text = input.value;
  if (!text.trim()) return;
  showPreview("Traduciendo…");
  const res = await window.wct.translateOutgoing(text, outLang);
  if (!res.ok) {
    showPreview(`No se pudo traducir: ${res.error}`, "err");
    return;
  }
  showPreview(`✓ Copiado: «${res.text}». En WoW: Enter → Ctrl+V → Enter`, "ok");
  input.value = "";
  input.blur();
  window.wct.blur();
});

window.wct.onComposeFocus(() => input.focus());

// Resize grip -------------------------------------------------------------------
// Native resize does not work on transparent frameless windows.

const grip = $("grip");
let drag = null;
grip.addEventListener("pointerdown", (e) => {
  drag = { x: e.screenX, y: e.screenY, w: window.outerWidth, h: window.outerHeight };
  grip.setPointerCapture(e.pointerId);
});
grip.addEventListener("pointermove", (e) => {
  if (!drag) return;
  window.wct.resize({ width: drag.w + e.screenX - drag.x, height: drag.h + e.screenY - drag.y });
});
grip.addEventListener("pointerup", (e) => {
  if (!drag) return;
  drag = null;
  grip.releasePointerCapture(e.pointerId);
  window.wct.resizeEnd();
});

// Init --------------------------------------------------------------------------

renderTabs();
renderEmpty();
window.wct.getPrefs().then((p) => {
  prefs = p;
  applyPrefs();
});

window.wct.onTranslatorChanged((label) => { $("provider-label").textContent = label; });
window.wct.getTranslator().then((t) => { $("provider-label").textContent = t.label; });
