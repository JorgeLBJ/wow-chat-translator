// Remembers recently seen messages so each one is processed only once, even though
// the panel is OCR'd again on every change and OCR output varies slightly between frames.

const normalize = (s) => s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "");

function bigrams(s) {
  const out = new Map();
  for (let i = 0; i < s.length - 1; i++) {
    const g = s.slice(i, i + 2);
    out.set(g, (out.get(g) ?? 0) + 1);
  }
  return out;
}

// Sørensen–Dice similarity over character bigrams, 0..1.
export function similarity(a, b) {
  if (a === b) return 1;
  if (a.length < 2 || b.length < 2) return 0;
  const A = bigrams(a);
  const B = bigrams(b);
  let overlap = 0;
  for (const [g, n] of A) overlap += Math.min(n, B.get(g) ?? 0);
  return (2 * overlap) / (a.length - 1 + (b.length - 1));
}

export function createTracker({ threshold = 0.6, max = 300 } = {}) {
  const seen = new Map(); // seq -> normalized player+text

  return {
    newMessages(messages) {
      const fresh = [];
      for (const msg of messages) {
        const key = normalize(`${msg.player}${msg.text}`);
        const prev = seen.get(msg.seq);
        // Same seq, same message when the text is similar, or when one reading starts
        // with the other (OCR glued a misread next line onto this one, or cut it short).
        if (prev !== undefined && (similarity(prev, key) >= threshold || prev.startsWith(key) || key.startsWith(prev))) {
          continue;
        }
        seen.delete(msg.seq);
        seen.set(msg.seq, key);
        if (seen.size > max) seen.delete(seen.keys().next().value);
        fresh.push(msg);
      }
      return fresh;
    },
  };
}
