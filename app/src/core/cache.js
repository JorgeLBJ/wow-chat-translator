// LRU cache in front of a translator. Failures are not cached so they can be retried.

export function createCachedTranslator(inner, { max = 1000 } = {}) {
  const cache = new Map();
  return {
    async translate(text, target) {
      const key = `${target}\u0000${text.trim().toLowerCase()}`;
      if (cache.has(key)) {
        const hit = cache.get(key);
        cache.delete(key);
        cache.set(key, hit);
        return hit;
      }
      const result = await inner.translate(text, target);
      cache.set(key, result);
      if (cache.size > max) cache.delete(cache.keys().next().value);
      return result;
    },
  };
}
