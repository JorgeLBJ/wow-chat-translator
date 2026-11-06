// Calls onExit once the game, after having been seen running, is no longer running.
// If the app starts before the game, it simply waits for the game to appear.
// Probe errors are ignored so a failed check never closes the app by mistake.

export function createGameWatcher({ isRunning, onExit }) {
  let seen = false;
  let done = false;
  return {
    async check() {
      if (done) return;
      let running;
      try {
        running = await isRunning();
      } catch {
        return;
      }
      if (running) {
        seen = true;
      } else if (seen) {
        done = true;
        onExit();
      }
    },
  };
}
