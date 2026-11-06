// Renders assets/icon.svg to the PNG sizes the app needs and packs a Windows .ico
// (PNG-compressed entries). Run with: npx electron scripts/build-icons.cjs
const { app, BrowserWindow } = require("electron");
const fs = require("node:fs");
const path = require("node:path");

const assets = path.join(__dirname, "..", "assets");
const svg = fs.readFileSync(path.join(assets, "icon.svg"), "utf8");
const ICO_SIZES = [16, 24, 32, 48, 64, 128, 256];

function buildIco(pngs) {
  // ICONDIR + ICONDIRENTRY[] + PNG data. Width/height 0 means 256.
  const header = Buffer.alloc(6 + 16 * pngs.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(pngs.length, 4);
  let offset = header.length;
  pngs.forEach(({ size, data }, i) => {
    const e = 6 + i * 16;
    header.writeUInt8(size >= 256 ? 0 : size, e);
    header.writeUInt8(size >= 256 ? 0 : size, e + 1);
    header.writeUInt8(0, e + 2);
    header.writeUInt8(0, e + 3);
    header.writeUInt16LE(1, e + 4);
    header.writeUInt16LE(32, e + 6);
    header.writeUInt32LE(data.length, e + 8);
    header.writeUInt32LE(offset, e + 12);
    offset += data.length;
  });
  return Buffer.concat([header, ...pngs.map((p) => p.data)]);
}

app.on("window-all-closed", () => {});
app.whenReady().then(async () => {
  const pngs = [];
  for (const size of [...ICO_SIZES, 512]) {
    const win = new BrowserWindow({ width: size, height: size, show: false, frame: false, transparent: true, useContentSize: true });
    const html = `<html><body style="margin:0;background:transparent"><img src="data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}" width="${size}" height="${size}"></body></html>`;
    await win.loadURL("data:text/html;base64," + Buffer.from(html).toString("base64"));
    await new Promise((r) => setTimeout(r, 150));
    const img = await win.webContents.capturePage({ x: 0, y: 0, width: size, height: size });
    const data = img.resize({ width: size, height: size }).toPNG();
    fs.writeFileSync(path.join(assets, `icon-${size}.png`), data);
    if (ICO_SIZES.includes(size)) pngs.push({ size, data });
    win.destroy();
  }
  fs.writeFileSync(path.join(assets, "icon.ico"), buildIco(pngs));
  console.log("icons written:", [...ICO_SIZES, 512].map((s) => `icon-${s}.png`).join(", "), "+ icon.ico");
  app.quit();
});
