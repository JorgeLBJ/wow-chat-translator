// Polls whether the game is running and reports transitions:
// onStart when it appears, onExit when it closes. Works for repeated sessions.
// Probe errors are ignored so a failed check never counts as the game closing.

export function createGameWatcher({ isRunning, onStart, onExit }) {
  let running = false;
  return {
    get running() {
      return running;
    },
    async check() {
      let now;
      try {
        now = await isRunning();
      } catch {
        return;
      }
      if (now && !running) {
        running = true;
        onStart();
      } else if (!now && running) {
        running = false;
        onExit();
      }
    },
  };
}
