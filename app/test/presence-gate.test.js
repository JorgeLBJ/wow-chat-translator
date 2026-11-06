import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createPresenceGate } from "../src/core/presence-gate.js";

describe("presence gate (hide only after a sustained absence)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const setup = (focused = false) => {
    const changes = [];
    const gate = createPresenceGate({ delayMs: 2000, isFocused: () => focused, onChange: (p) => changes.push(p) });
    return { gate, changes };
  };

  it("shows right away when the strip appears", () => {
    const { gate, changes } = setup();
    gate.seen();
    expect(changes).toEqual([true]);
  });

  it("does not hide on a short absence", () => {
    const { gate, changes } = setup();
    gate.seen();
    gate.lost();
    vi.advanceTimersByTime(1500);
    gate.seen();
    vi.advanceTimersByTime(5000);
    expect(changes).toEqual([true]);
  });

  it("hides after the strip has been gone for the whole delay", () => {
    const { gate, changes } = setup();
    gate.seen();
    gate.lost();
    vi.advanceTimersByTime(2000);
    expect(changes).toEqual([true, false]);
  });

  it("never hides while the window has the focus (user is typing)", () => {
    const { gate, changes } = setup(true);
    gate.seen();
    gate.lost();
    vi.advanceTimersByTime(10000);
    expect(changes).toEqual([true]);
  });

  it("cannot flip-flop faster than the delay (the reported flicker loop)", () => {
    const { gate, changes } = setup();
    gate.seen();
    for (let i = 0; i < 40; i++) {
      gate.lost();
      vi.advanceTimersByTime(60);
      gate.seen();
      vi.advanceTimersByTime(60);
    }
    expect(changes).toEqual([true]);
  });

  it("does not repeat a state it already reported", () => {
    const { gate, changes } = setup();
    gate.seen();
    gate.seen();
    expect(changes).toEqual([true]);
  });
});
