//! Reports when the game starts and exits (port of `app/src/core/game-watcher.js`).
//! The caller probes `Wow.exe` every few seconds and feeds the result here.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameChange {
    Started,
    Exited,
}

#[derive(Debug, Default, Clone)]
pub struct GameWatcher {
    running: bool,
}

impl GameWatcher {
    pub fn running(&self) -> bool {
        self.running
    }

    /// Feeds one probe; returns the transition it caused, if any.
    pub fn update(&mut self, now_running: bool) -> Option<GameChange> {
        match (self.running, now_running) {
            (false, true) => {
                self.running = true;
                Some(GameChange::Started)
            }
            (true, false) => {
                self.running = false;
                Some(GameChange::Exited)
            }
            _ => None,
        }
    }
}
