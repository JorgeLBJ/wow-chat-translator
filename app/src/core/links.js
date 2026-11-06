// Item, spell, quest, enchant... links arrive as "[Name]" (the addon writes them as
// "[ Name ]" so OCR reads the brackets reliably). They must never be translated:
// they are swapped for numbered placeholders before translating and put back after.
// Google and LLMs both keep "[0]"-style tokens intact (verified against Google).

const LINK = /\[\s*([^[\]]+?)\s*\]/g;
const PLACEHOLDER = /\[\s*(\d+)\s*\]/g;

export function tidyLinks(text) {
  return text.replace(LINK, (_m, name) => `[${name}]`);
}

export function protectLinks(text) {
  const links = [];
  const masked = text.replace(LINK, (_m, name) => {
    links.push(`[${name}]`);
    return `[${links.length - 1}]`;
  });
  return { text: masked, links };
}

export function restoreLinks(text, links) {
  return text.replace(PLACEHOLDER, (m, i) => links[Number(i)] ?? m);
}

// True when nothing but placeholders and punctuation is left to translate.
export function onlyLinks(masked) {
  return !/\p{L}/u.test(masked.replace(PLACEHOLDER, ""));
}
