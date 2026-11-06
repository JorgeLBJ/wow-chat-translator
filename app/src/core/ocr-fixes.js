// Fixes common OCR misreads of chat words before translating. With the panel font,
// lowercase "l" and "1", and "o" and "0", look almost identical: "lol" is read as "101".
//
// Conservative on purpose: a pure "101" is only fixed when it is the whole message
// (it can be a real number mid-sentence: "level 101"); readings that mix letters and
// digits ("1o1", "IoI", "l0l") are never numbers, so they are always fixed.

const LOL_MIXED = /(^|[\s(])([1lI|][0oO][1lI|])(?=$|[\s!?.,:;)])/g;
const ONLY_101 = /^\s*101(\s*[!?.]*)\s*$/;

export function fixOcrWords(text) {
  if (ONLY_101.test(text)) return text.replace(ONLY_101, "lol$1");
  return text.replace(LOL_MIXED, (match, lead, word) => (/^\d+$/.test(word) ? match : `${lead}lol`));
}
