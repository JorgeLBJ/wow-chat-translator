//! Decides when the overlay follows the addon strip (port of
//! `app/src/core/presence-gate.js`). Showing is immediate; hiding only happens after the
//! strip has been gone for `delay_ms` in a row, and never while the overlay has the
//! keyboard focus (the user is typing in it).
//!
//! Without this, focusing the overlay over a fullscreen WoW made the game stop drawing,
//! the strip vanished, the overlay hid, WoW came back, the overlay showed again... a
//! hide/show loop several times per second that froze the screen.
//!
//! Pure and clock-free: the caller passes the current time (milliseconds) and polls.

#[derive(Debug, Clone)]
pub struct PresenceGate {
    delay_ms: u64,
    /// Last reported presence; `None` before the first report.
    state: Option<bool>,
    /// When the pending hide is due; `None` when no hide is pending.
    deadline: Option<u64>,
}

impl PresenceGate {
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms, state: None, deadline: None }
    }

    /// The strip is visible. Returns `Some(true)` when this is a change to report.
    pub fn seen(&mut self, _now: u64) -> Option<bool> {
        self.deadline = None;
        self.report(true)
    }

    /// The strip is not visible: starts the delay unless it is already running or the
    /// overlay is already hidden. Never reports anything by itself.
    pub fn lost(&mut self, now: u64) {
        if self.deadline.is_some() || self.state == Some(false) {
            return;
        }
        self.deadline = Some(now + self.delay_ms);
    }

    /// Call regularly. Returns `Some(false)` once the delay has passed without the strip,
    /// unless the window has the focus, in which case the delay starts over.
    pub fn poll(&mut self, now: u64, focused: bool) -> Option<bool> {
        self.deadline.filter(|&d| now >= d)?;
        if focused {
            self.deadline = Some(now + self.delay_ms); // try again later; never hide under the user
            return None;
        }
        self.deadline = None;
        self.report(false)
    }

    /// Last reported presence.
    pub fn state(&self) -> Option<bool> {
        self.state
    }

    fn report(&mut self, present: bool) -> Option<bool> {
        if self.state == Some(present) {
            return None;
        }
        self.state = Some(present);
        Some(present)
    }
}
