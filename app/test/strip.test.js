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
