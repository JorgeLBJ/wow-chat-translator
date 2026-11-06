// Finds and decodes the addon's pixel strip inside a captured RGB patch taken from the
// top-left corner of the WoW window. The cell size (1-4 physical px) and a small offset
// (0-2 px, if WoW draws the strip slightly shifted) are discovered automatically; the
// layout that worked last time is tried first.

import { COLS, ROWS, decodeFrame } from "./strip.js";

const CELL_SIZES = [2, 3, 4, 1];
const OFFSETS = [0, 1, 2];

function sample(px, w, h, cell, dx, dy) {
  const half = Math.floor(cell / 2);
  const colors = [];
  for (let i = 0; i < COLS * ROWS; i++) {
    const x = dx + (i % COLS) * cell + half;
    const y = dy + Math.floor(i / COLS) * cell + half;
    if (x >= w || y >= h) return null;
    const o = (y * w + x) * 3;
    colors.push([px[o], px[o + 1], px[o + 2]]);
  }
  return colors;
}

// Cheap test before a full decode: the first marker cell is strong red + some blue,
// with no green (levels 15, 0, 8).
function looksLikeMarker(px, w, cell, dx, dy) {
  const half = Math.floor(cell / 2);
  const o = ((dy + half) * w + dx + half) * 3;
  return px[o] > 150 && px[o + 1] < 100;
}

// px: Uint8Array of w*h RGB triplets. hint: optional object remembering the layout.
// Returns { seq, message } (message null for the idle frame) or null.
export function findStrip(px, w, h, hint = {}) {
  const tries = [];
  if (hint.cell) tries.push([hint.cell, hint.dx, hint.dy]);
  for (const cell of CELL_SIZES) for (const dy of OFFSETS) for (const dx of OFFSETS) tries.push([cell, dx, dy]);
  for (const [cell, dx, dy] of tries) {
    if ((dx + COLS * cell) > w || (dy + ROWS * cell) > h) continue;
    if (!looksLikeMarker(px, w, cell, dx, dy)) continue;
    const colors = sample(px, w, h, cell, dx, dy);
    const result = colors && decodeFrame(colors);
    if (result) {
      Object.assign(hint, { cell, dx, dy });
      return result;
    }
  }
  return null;
}
