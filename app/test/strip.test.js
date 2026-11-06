import { describe, it, expect } from "vitest";
import { CELLS, encodeFrame, decodeFrame, buildPayload, parsePayload, MAX_PAYLOAD } from "../src/core/strip.js";

const msg = { channel: "Trade", cls: "PALADIN", player: "La Muertita", text: "WTS [Blacksmithing: Runed Copper Bracers] 5g, lol ñandú 😀" };
// What the screen shows: each level drawn as level * 17.
const toColors = (levels, fn = (v) => v) => levels.map(([r, g, b]) => [fn(r * 17), fn(g * 17), fn(b * 17)]);

describe("strip protocol", () => {
  it("round-trips a chat message exactly, including links, accents and emoji", () => {
    const frame = encodeFrame(7, buildPayload(msg));
    expect(frame).toHaveLength(CELLS);
    expect(decodeFrame(toColors(frame))).toEqual({ seq: 7, message: msg });
  });

  it("survives color noise thanks to the calibration ramp", () => {
    let i = 0;
    const noise = (v) => Math.max(0, Math.min(255, v + ((i++ * 37) % 13) - 6)); // deterministic ±6
    expect(decodeFrame(toColors(encodeFrame(3, buildPayload(msg)), noise))?.message).toEqual(msg);
  });

  it("survives a darker or gamma-shifted screen thanks to the calibration ramp", () => {
    const dark = (v) => Math.round(255 * Math.pow(v / 255, 1.25) * 0.85);
    expect(decodeFrame(toColors(encodeFrame(9, buildPayload(msg)), dark))?.message).toEqual(msg);
  });

  it("rejects an area that is not a strip (no marker)", () => {
    const scenery = Array.from({ length: CELLS }, (_, k) => [(k * 7) % 255, (k * 13) % 255, (k * 29) % 255]);
    expect(decodeFrame(scenery)).toBeNull();
  });

  it("rejects a frame whose payload was corrupted (checksum)", () => {
    const frame = encodeFrame(1, buildPayload(msg));
    frame[30] = [(frame[30][0] + 5) % 16, frame[30][1], frame[30][2]];
    expect(decodeFrame(toColors(frame))).toBeNull();
  });

  it("wraps the sequence number at 12 bits", () => {
    expect(decodeFrame(toColors(encodeFrame(4096 + 5, buildPayload(msg))))?.seq).toBe(5);
  });

  it("truncates text that would not fit, without breaking UTF-8", () => {
    const long = { ...msg, text: "á".repeat(400) };
    const bytes = buildPayload(long);
    expect(bytes.length).toBeLessThanOrEqual(MAX_PAYLOAD);
    const back = parsePayload(bytes);
    expect(back.text.length).toBeGreaterThan(200);
    expect(back.text).toMatch(/^á+$/);
  });

  it("keeps an unknown class empty", () => {
    const back = parsePayload(buildPayload({ ...msg, cls: null }));
    expect(back.cls).toBeNull();
  });
});

describe("strip idle frame", () => {
  it("decodes an empty payload as an alive strip with no message", () => {
    const frame = encodeFrame(42, new Uint8Array(0));
    expect(decodeFrame(toColors(frame))).toEqual({ seq: 42, message: null });
  });
});

import { findStrip } from "../src/core/strip-reader.js";

// Draws a frame into an RGB patch the way WoW would: cell x cell pixels per cell,
// shifted by (dx, dy), over a scenery background.
function paint(frame, { cell = 2, dx = 0, dy = 0, w = 260, h = 28, edgeBlur = false } = {}) {
  const px = new Uint8Array(w * h * 3);
  for (let i = 0; i < px.length; i++) px[i] = (i * 31) % 200; // scenery
  frame.forEach(([r, g, b], i) => {
    const cx = (i % 64) * cell + dx;
    const cy = Math.floor(i / 64) * cell + dy;
    for (let y = 0; y < cell; y++) for (let x = 0; x < cell; x++) {
      const o = ((cy + y) * w + (cx + x)) * 3;
      const blur = edgeBlur && (x === 0 || y === 0) ? 0.5 : 1; // half-blended cell edges
      px[o] = r * 17 * blur + px[o] * (1 - blur);
      px[o + 1] = g * 17 * blur + px[o + 1] * (1 - blur);
      px[o + 2] = b * 17 * blur + px[o + 2] * (1 - blur);
    }
  });
  return { px, w, h };
}

describe("findStrip", () => {
  const frame = encodeFrame(77, buildPayload(msg));

  it.each([
    [{ cell: 2 }],
    [{ cell: 3, dx: 1 }],
    [{ cell: 4, dy: 2 }],
    [{ cell: 1, dx: 2, dy: 1 }],
    [{ cell: 3, edgeBlur: true }],
  ])("finds and decodes the strip (%o)", (opts) => {
    const { px, w, h } = paint(frame, opts);
    expect(findStrip(px, w, h)?.message).toEqual(msg);
  });

  it("returns null when there is no strip", () => {
    const w = 260, h = 28;
    const px = new Uint8Array(w * h * 3).map((_, i) => (i * 13) % 251);
    expect(findStrip(px, w, h)).toBeNull();
  });

  it("tries the last working layout first", () => {
    const { px, w, h } = paint(frame, { cell: 3, dx: 1 });
    const hint = {};
    findStrip(px, w, h, hint);
    expect(hint.cell).toBe(3);
    expect(findStrip(px, w, h, hint)?.message).toEqual(msg);
  });
});
