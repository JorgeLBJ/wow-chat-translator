// Item, spell, quest, enchant... links arrive as "[Name]" (the addon writes them as
// "[ Name ]" so OCR reads the brackets reliably). They must never be translated:
// they are swapped for numbered placeholders before translating and put back after.
// Google and LLMs both keep "[0]"-style tokens intact (verified against Google).

const LINK = /\[\s*([^[\]]+?)\s*\]/g;
const PLACEHOLDER = /\[\s*(\d+)\s*\]/g;

// Leftover of WoW named link colors ("|cnIQ1:" rendered as "/cnlQ1 ") from addon
// versions that did not strip them; OCR may read I as l and drop the colon.
const COLOR_LEFTOVER = /\/cn[A-Za-z0-9]{2,6}:?\s*/g;

// The addon writes links as "< Name >": Windows OCR reads "<" and ">" reliably, while
// "]" at a line end or between two links is often misread or dropped (measured).
// The spaces are required, so "<3" or "> 5g" typed by players are not links.
const ANGLE_LINK = /<\s+([^<>[\]]+?)\s+>/g;

export function tidyLinks(text) {
  return text
    .replace(COLOR_LEFTOVER, "")
    .replace(ANGLE_LINK, (_m, name) => `[${name}]`)
    .replace(LINK, (_m, name) => `[${name}]`)
    .replace(/\s{2,}/g, " ")
    .trim();
}

export function protectLinks(text) {
  const links = [];
  const masked = text.replace(LINK, (_m, name) => {
    links.push(`[${name}]`);
    return `[${links.length - 1}]`;
  });
  return { text: masked, links };
}

// Pixel-strip text: the addon wraps real WoW links (|H...|h[Name]|h) in \x02 ... \x03,
// characters players cannot type. Only those are kept untranslated; brackets a player
// typed by hand are ordinary text.
const MARKED_LINK = /\x02([^\x02\x03]*)\x03/g;

export function stripLinkMarkers(text) {
  return text.replace(/[\x02\x03]/g, "");
}

export function protectMarkedLinks(text) {
  const links = [];
  const masked = text.replace(MARKED_LINK, (_m, name) => {
    links.push(name);
    return `[${links.length - 1}]`;
  });
  return { text: stripLinkMarkers(masked), links };
}

export function restoreLinks(text, links) {
  return text.replace(PLACEHOLDER, (m, i) => links[Number(i)] ?? m);
}

// True when nothing but placeholders and punctuation is left to translate.
export function onlyLinks(masked) {
  return !/\p{L}/u.test(masked.replace(PLACEHOLDER, ""));
}
