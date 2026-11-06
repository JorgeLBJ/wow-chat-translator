// Pixel-strip protocol: how the addon sends a chat message as a small grid of colored
// cells, and how the app decodes it exactly (no OCR).
//
// Design informed by Parley's documented protocol (MIT,
// https://github.com/Duhlude/parley/blob/main/docs/HOW_IT_WORKS.md) and LibSerpix (MIT).
// The Lua encoder in the addon must produce exactly what encodeFrame() produces.
//
// Layout (COLS x ROWS cells, read row by row):
//   cells 0-3    marker
//   cells 4-19   calibration ramp, cell 4+i = (i, i, i)
//   cell  20     sequence number (12 bits)
//   cell  21     payload length in bytes (12 bits)
//   cells 22-23  checksum: two 12-bit running sums mod 4095
//   cells 24+    payload bytes, as 4-bit nibbles (high first), 3 nibbles per cell
// Every cell carries three 4-bit levels (R, G, B), drawn on screen as level * 17.

export const COLS = 64;
export const ROWS = 6;
export const CELLS = COLS * ROWS;
export const MARKER = [
  [15, 0, 8],
  [0, 15, 15],
  [15, 8, 0],
  [3, 0, 15],
];
const CAL = 4;
const SEQ = 20;
const LEN = 21;
const SUM = 22;
const DATA = 24;
export const MAX_PAYLOAD = Math.floor(((CELLS - DATA) * 3) / 2);
const SEP = "\x1F";
const VERSION = "v1";

const enc = new TextEncoder();
const dec = new TextDecoder("utf-8", { fatal: true });

const to12 = (v) => [(v >> 8) & 15, (v >> 4) & 15, v & 15];
const from12 = ([a, b, c]) => (a << 8) | (b << 4) | c;

function checksum(bytes) {
  let s1 = 0;
  let s2 = 0;
  for (const b of bytes) {
    s1 = (s1 + b) % 4095;
    s2 = (s2 + s1) % 4095;
  }
  return [s1, s2];
}

// Least-squares fit y = a + b*x + c*x^2 over x = 0..15; returns the fitted 16 values.
function fitQuadratic(ys) {
  const n = ys.length;
  let s0 = 0, s1 = 0, s2 = 0, s3 = 0, s4 = 0, t0 = 0, t1 = 0, t2 = 0;
  for (let x = 0; x < n; x++) {
    const y = ys[x];
    const x2 = x * x;
    s0 += 1; s1 += x; s2 += x2; s3 += x2 * x; s4 += x2 * x2;
    t0 += y; t1 += x * y; t2 += x2 * y;
  }
  // Solve the 3x3 normal equations with Cramer's rule.
  const det3 = (m) =>
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) -
    m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) +
    m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
  const M = [[s0, s1, s2], [s1, s2, s3], [s2, s3, s4]];
  const T = [t0, t1, t2];
  const d = det3(M);
  const col = (k) => det3(M.map((row, r) => row.map((v, c) => (c === k ? T[r] : v))));
  const [a, b, c] = [col(0) / d, col(1) / d, col(2) / d];
  return ys.map((_, x) => a + b * x + c * x * x);
}

// ---- payload -----------------------------------------------------------------

export function buildPayload({ channel, cls, player, text }) {
  const head = enc.encode([VERSION, channel, cls ?? "", player, ""].join(SEP));
  const budget = MAX_PAYLOAD - head.length;
  let body = enc.encode(text);
  if (body.length > budget) {
    // Cut by whole characters so the UTF-8 stays valid.
    const chars = Array.from(text);
    while (enc.encode(chars.join("")).length > budget) chars.pop();
    body = enc.encode(chars.join(""));
  }
  const out = new Uint8Array(head.length + body.length);
  out.set(head);
  out.set(body, head.length);
  return out;
}

export function parsePayload(bytes) {
  let s;
  try {
    s = dec.decode(bytes);
  } catch {
    return null;
  }
  const parts = s.split(SEP);
  if (parts.length < 5 || parts[0] !== VERSION) return null;
  const [, channel, cls, player, ...rest] = parts;
  return { channel, cls: cls || null, player, text: rest.join(SEP) };
}

// ---- frames ------------------------------------------------------------------

// Returns CELLS cells of [r, g, b] levels (0-15).
export function encodeFrame(seq, payload) {
  if (payload.length > MAX_PAYLOAD) throw new Error("payload too long");
  const cells = Array.from({ length: CELLS }, () => [0, 0, 0]);
  MARKER.forEach((c, i) => { cells[i] = [...c]; });
  for (let i = 0; i < 16; i++) cells[CAL + i] = [i, i, i];
  cells[SEQ] = to12(seq % 4096);
  cells[LEN] = to12(payload.length);
  const [s1, s2] = checksum(payload);
  cells[SUM] = to12(s1);
  cells[SUM + 1] = to12(s2);
  const nibbles = [];
  for (const b of payload) nibbles.push(b >> 4, b & 15);
  for (let i = 0; i < nibbles.length; i += 3) {
    cells[DATA + i / 3] = [nibbles[i], nibbles[i + 1] ?? 0, nibbles[i + 2] ?? 0];
  }
  return cells;
}

// colors: CELLS samples of [r, g, b] (0-255) taken at each cell center.
// Returns { seq, message } or null when the area is not a valid frame.
export function decodeFrame(colors) {
  if (!colors || colors.length < CELLS) return null;
  // Learn how each of the 16 levels looks on this screen, per channel. A quadratic
  // fit over the 16 calibration samples smooths their noise and still follows a
  // gamma-shifted or darker screen.
  const raw = [0, 1, 2].map((ch) => Array.from({ length: 16 }, (_, i) => colors[CAL + i][ch]));
  if (raw.some((r) => r[15] - r[0] < 60)) return null;
  const ref = raw.map(fitQuadratic);
  const level = (v, ch) => {
    let best = 0;
    for (let i = 1; i < 16; i++) if (Math.abs(v - ref[ch][i]) < Math.abs(v - ref[ch][best])) best = i;
    return best;
  };
  const cell = (i) => colors[i].map((v, ch) => level(v, ch));

  for (let i = 0; i < MARKER.length; i++) {
    const c = cell(i);
    if (c[0] !== MARKER[i][0] || c[1] !== MARKER[i][1] || c[2] !== MARKER[i][2]) return null;
  }
  const seq = from12(cell(SEQ));
  const length = from12(cell(LEN));
  if (length > MAX_PAYLOAD) return null;
  const expected = [from12(cell(SUM)), from12(cell(SUM + 1))];

  const nibbles = [];
  for (let i = DATA; nibbles.length < length * 2; i++) nibbles.push(...cell(i));
  const bytes = new Uint8Array(length);
  for (let i = 0; i < length; i++) bytes[i] = (nibbles[2 * i] << 4) | nibbles[2 * i + 1];

  const [s1, s2] = checksum(bytes);
  if (s1 !== expected[0] || s2 !== expected[1]) return null;
  // Empty payload = idle frame: the addon is running but has no message to send.
  if (length === 0) return { seq, message: null };
  const message = parsePayload(bytes);
  return message ? { seq, message } : null;
}
