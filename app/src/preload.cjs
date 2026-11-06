const { contextBridge, ipcRenderer } = require("electron");

const on = (channel) => (cb) => ipcRenderer.on(channel, (_e, payload) => cb(payload));

contextBridge.exposeInMainWorld("wct", {
  onAdd: on("chat:add"),
  onUpdate: on("chat:update"),
  onToggle: on("view:toggle"),
  onStatus: on("status"),
  onOcrState: on("ocr:state"),
  onMaximized: on("win:maximized"),
  onComposeFocus: on("compose:focus"),
  calibrate: () => ipcRenderer.send("app:calibrate"),
  close: () => ipcRenderer.send("app:close"),
  minimize: () => ipcRenderer.send("win:minimize"),
  maximize: () => ipcRenderer.send("win:maximize"),
  dock: () => ipcRenderer.send("win:dock"),
  blur: () => ipcRenderer.send("win:blur"),
  resize: (size) => ipcRenderer.send("win:resize", size),
  resizeEnd: () => ipcRenderer.send("win:resize-end"),
  getPrefs: () => ipcRenderer.invoke("prefs:get"),
  setPrefs: (patch) => ipcRenderer.invoke("prefs:set", patch),
  translateOutgoing: (text, lang) => ipcRenderer.invoke("compose:translate", { text, lang }),
  calibDone: (rect) => ipcRenderer.send("calib:done", rect),
  calibCancel: () => ipcRenderer.send("calib:cancel"),
});
