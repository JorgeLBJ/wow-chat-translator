// Turns :shortcodes: that players type out of Discord/Slack habit (":sob:", ":joy:")
// into the emoji they stand for, using gemoji (GitHub's shortcode list, MIT).
// Done before translating, so translators see the emoji instead of a word like "sob".

import { gemoji } from "gemoji";

const BY_NAME = new Map();
for (const { emoji, names } of gemoji) for (const name of names) BY_NAME.set(name, emoji);

// Not touching letters or digits on either side, so times like 12:30:45 are left alone.
const SHORTCODE = /(?<![\p{L}\p{N}]):([a-z0-9_+-]+):(?![\p{L}\p{N}])/giu;

export function replaceShortcodes(text) {
  return text.replace(SHORTCODE, (match, name) => BY_NAME.get(name.toLowerCase()) ?? match);
}
