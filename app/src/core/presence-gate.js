// Decides when the overlay follows the addon strip. Showing is immediate; hiding
// only happens after the strip has been gone for `delayMs` in a row, and never while
// the overlay has the focus (the user is typing in it).
//
// Without this, focusing the overlay (Ctrl+Shift+Enter) made a fullscreen WoW stop
// drawing, the strip vanished, the overlay hid, WoW came back, the overlay showed
// again... a hide/show loop several times per second that froze the screen.

export function createPresenceGate({ delayMs = 2000, isFocused = () => false, onChange }) {
  let state = null;
  let timer = null;

  const report = (present) => {
    if (state === present) return;
    state = present;
    onChange(present);
  };

  return {
    seen() {
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      report(true);
    },
    lost() {
      if (timer || state === false) return;
      timer = setTimeout(function check() {
        if (isFocused()) {
          timer = setTimeout(check, delayMs); // try again later; never hide under the user
          return;
        }
        timer = null;
        report(false);
      }, delayMs);
    },
  };
}
