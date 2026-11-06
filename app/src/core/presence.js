// Debounced "is the addon panel on screen?" state. The first observation is reported
// right away; after that a change is reported only when it holds for `threshold`
// consecutive observations, so loading screens or a single bad frame do not flicker.

export function createPresence({ threshold = 3, onChange }) {
  let state = null;
  let streak = 0;
  return {
    get present() {
      return state;
    },
    update(seen) {
      if (state === null) {
        state = seen;
        onChange(state);
        return;
      }
      if (seen === state) {
        streak = 0;
        return;
      }
      streak++;
      if (streak >= threshold) {
        state = seen;
        streak = 0;
        onChange(state);
      }
    },
  };
}
