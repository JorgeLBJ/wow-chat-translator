// Parses OCR output of the addon panel. Each message starts with
// "NNN Channel CLASS Player Name: message" (CLASS is optional for older addon versions);
// wrapped lines continue the previous message.

const START = /^([0-9OoIl|()]{3})\s+(\S+)\s+(.+?):\s*(.*)$/;
// A clipped "0" at the left edge of the capture is often read as "(" or ")".
const SEQ_FIX = { O: "0", o: "0", "(": "0", ")": "0", I: "1", l: "1", "|": "1" };

export const CLASSES = [
  "WARRIOR", "PALADIN", "HUNTER", "ROGUE", "PRIEST", "SHAMAN", "MAGE", "WARLOCK", "DRUID",
  "DEATHKNIGHT", "MONK", "DEMONHUNTER", "EVOKER",
];
const NO_CLASS = "NOCLASS";

function toSeq(raw) {
  // Require at least one real digit so words like "lol" are not taken as a sequence.
  if (!/\d/.test(raw)) return null;
  return Number(raw.replace(/[OoIl|()]/g, (c) => SEQ_FIX[c]));
}

function distance(a, b) {
  const dp = Array.from({ length: a.length + 1 }, (_, i) => [i]);
  for (let j = 1; j <= b.length; j++) dp[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    }
  }
  return dp[a.length][b.length];
}

// Returns a class name, NO_CLASS, or undefined when the token is not a class token.
function readClass(token) {
  const fixed = token.replace(/l/g, "I").replace(/0/g, "O").replace(/1/g, "I");
  if (fixed.length < 4 || !/^[A-Z]+$/.test(fixed)) return undefined;
  for (const name of [...CLASSES, NO_CLASS]) {
    if (distance(fixed, name) <= (name.length > 5 ? 2 : 1)) return name;
  }
  return undefined;
}

function splitClass(who) {
  const [first, ...rest] = who.split(/\s+/);
  if (rest.length === 0) return { cls: null, player: who };
  const cls = readClass(first);
  if (cls === undefined) return { cls: null, player: who };
  return { cls: cls === NO_CLASS ? null : cls, player: rest.join(" ") };
}

export function parseLines(lines) {
  const messages = [];
  let current = null;
  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) continue;
    const match = START.exec(line);
    const seq = match ? toSeq(match[1]) : null;
    if (match && seq !== null) {
      const { cls, player } = splitClass(match[3].trim());
      current = { seq, channel: match[2], cls, player, text: match[4].trim() };
      messages.push(current);
    } else if (current) {
      current.text = `${current.text} ${line}`.trim();
    }
    // A continuation with no visible first line belongs to a message that scrolled away.
  }
  return messages;
}
