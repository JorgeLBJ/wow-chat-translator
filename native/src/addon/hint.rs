//! When to look for an outdated addon: the game has been running for a while and the
//! strip has not appeared once. Checked at most once per game session.

#[derive(Debug, Clone)]
pub struct UpdateHint {
    delay_ms: u64,
    started_at: Option<u64>,
    seen: bool,
    checked: bool,
}

impl UpdateHint {
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms, started_at: None, seen: false, checked: false }
    }

    pub fn game_started(&mut self, now_ms: u64) {
        *self = Self { started_at: Some(now_ms), ..Self::new(self.delay_ms) };
    }

    pub fn game_exited(&mut self) {
        *self = Self::new(self.delay_ms);
    }

    pub fn strip_seen(&mut self) {
        self.seen = true;
    }

    /// True once per session, `delay_ms` after the game started without any strip.
    pub fn due(&mut self, now_ms: u64) -> bool {
        let Some(start) = self.started_at else { return false };
        if self.seen || self.checked || now_ms.saturating_sub(start) < self.delay_ms {
            return false;
        }
        self.checked = true;
        true
    }
}
