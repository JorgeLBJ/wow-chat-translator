// Tries the primary translator and, if it fails, the secondary one, so a down LLM
// server never leaves chat untranslated. The result says which provider answered.

export function createFallbackTranslator(primary, secondary) {
  return {
    async translate(text, target) {
      try {
        return { ...(await primary.translator.translate(text, target)), provider: primary.name };
      } catch (err) {
        const res = await secondary.translator.translate(text, target);
        return { ...res, provider: secondary.name, fallbackReason: err.message };
      }
    },
  };
}
