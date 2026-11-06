// Free, keyless Google Translate endpoint (unofficial, the one the Chrome dictionary
// extension uses). The classic `translate_a/single?client=gtx` endpoint answered 429
// from this network, so it is not used. Any object with
// translate(text, target) -> { text, source } can replace this one.

const ENDPOINT = "https://clients5.google.com/translate_a/t";

export function createGoogleTranslator({ fetchFn = globalThis.fetch } = {}) {
  return {
    async translate(text, target) {
      const url = new URL(ENDPOINT);
      url.searchParams.set("client", "dict-chrome-ex");
      url.searchParams.set("sl", "auto");
      url.searchParams.set("tl", target);
      url.searchParams.set("q", text);
      const res = await fetchFn(url.toString());
      if (!res.ok) throw new Error(`Google Translate HTTP ${res.status}`);
      const data = await res.json();
      // Shape: [["translated", "detectedSource"]] (or ["translated"] when sl is fixed).
      const first = Array.isArray(data) ? data[0] : undefined;
      if (Array.isArray(first)) {
        return { text: String(first[0] ?? ""), source: typeof first[1] === "string" ? first[1] : "auto" };
      }
      if (typeof first === "string") return { text: first, source: "auto" };
      throw new Error("Google Translate: unexpected response");
    },
  };
}
