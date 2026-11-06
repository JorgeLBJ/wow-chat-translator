const sel = document.getElementById("sel");
let start = null;

function rectFrom(e) {
  const x = Math.min(start.x, e.clientX);
  const y = Math.min(start.y, e.clientY);
  return { x, y, w: Math.abs(e.clientX - start.x), h: Math.abs(e.clientY - start.y) };
}

window.addEventListener("mousedown", (e) => {
  start = { x: e.clientX, y: e.clientY };
  sel.style.display = "block";
});

window.addEventListener("mousemove", (e) => {
  if (!start) return;
  const r = rectFrom(e);
  Object.assign(sel.style, { left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` });
});

window.addEventListener("mouseup", (e) => {
  if (!start) return;
  const r = rectFrom(e);
  start = null;
  if (r.w < 20 || r.h < 20) {
    sel.style.display = "none";
    return;
  }
  window.wct.calibDone(r);
});

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") window.wct.calibCancel();
});
