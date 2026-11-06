//! Background thread: follows `Wow.exe` (every 5 s) and, while it runs, captures the
//! strip patch every ~35 ms, decodes new frames into the pipeline and drives the
//! window through the presence gate. Port of the reader parts of `app/src/main.js`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::capture::{self, Capturer, Grab, PATCH_H, PATCH_W};
use crate::strip_reader::{find_strip, Layout};

use super::game_watcher::{GameChange, GameWatcher};
use super::presence::PresenceGate;
use super::shared::Shared;
use super::style::ReaderState;

const TICK: Duration = Duration::from_millis(35);
const IDLE_TICK: Duration = Duration::from_millis(250);
const GAME_CHECK: Duration = Duration::from_secs(5);
const HIDE_DELAY_MS: u64 = 2000;
const WAITING_FOR_WOW: &str = "Esperando a que abras WoW…";

/// What the last capture showed, to log and report only changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seen {
    Nothing,
    NoWindow,
    Absent,
    Present,
    Error,
}

struct Reader {
    shared: Arc<Shared>,
    capturer: Capturer,
    hint: Option<Layout>,
    last_hash: Option<u64>,
    last_seq: Option<u16>,
    seen: Seen,
}

impl Reader {
    fn reset(&mut self) {
        self.last_hash = None;
        self.last_seq = None;
        self.seen = Seen::Nothing;
    }

    fn enter(&mut self, seen: Seen, log: &str) {
        if self.seen != seen {
            self.seen = seen;
            self.shared.log.write(log);
        }
    }

    /// One capture. Returns the presence to feed to the gate, if any.
    fn tick(&mut self) -> Option<bool> {
        match self.capturer.grab() {
            Ok(Grab::WowNotFound) => {
                self.last_hash = None;
                self.enter(Seen::NoWindow, "wow window not found");
                self.shared.set_reader(ReaderState::Idle);
                None
            }
            Ok(Grab::Minimized) => {
                self.last_hash = None;
                self.enter(Seen::Absent, "strip absent (wow minimized)");
                self.shared.set_reader(ReaderState::NoStrip);
                Some(false)
            }
            Ok(Grab::Patch(px)) => {
                let hash = fnv1a(&px);
                if self.last_hash == Some(hash) {
                    // Same pixels as last time: same answer, nothing to decode.
                    return Some(self.seen == Seen::Present);
                }
                self.last_hash = Some(hash);
                let Some(frame) = find_strip(&px, PATCH_W, PATCH_H, &mut self.hint) else {
                    self.enter(Seen::Absent, "strip absent");
                    self.shared.set_reader(ReaderState::NoStrip);
                    return Some(false);
                };
                self.enter(Seen::Present, "strip visible");
                self.shared.set_reader(ReaderState::Ok);
                self.shared.set_status_text("");
                if self.last_seq != Some(frame.seq) {
                    self.last_seq = Some(frame.seq);
                    if let Some(msg) = frame.message {
                        self.shared.log.write(&format!(
                            "strip {{\"seq\":{},\"cell\":{},\"channel\":{:?}}}",
                            frame.seq,
                            self.hint.map_or(0, |l| l.cell),
                            msg.channel
                        ));
                        self.shared.add_messages(&[msg]);
                    }
                }
                Some(true)
            }
            Err(e) => {
                self.last_hash = None;
                self.enter(Seen::Error, &format!("capture error {e}"));
                self.shared.set_reader(ReaderState::Error);
                self.shared.set_status_text(&format!("Lector: {e}"));
                None
            }
        }
    }
}

pub fn spawn(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("strip-reader".into()).spawn(move || run(shared));
}

fn run(shared: Arc<Shared>) {
    let start = Instant::now();
    let now_ms = || start.elapsed().as_millis() as u64;
    let mut watcher = GameWatcher::default();
    let mut gate = PresenceGate::new(HIDE_DELAY_MS);
    let mut reader = Reader {
        shared: shared.clone(),
        capturer: Capturer::new(),
        hint: None,
        last_hash: None,
        last_seq: None,
        seen: Seen::Nothing,
    };
    let mut next_check = Instant::now();
    shared.set_reader(ReaderState::Idle);
    shared.set_status_text(WAITING_FOR_WOW);

    loop {
        if shared.instance.as_ref().is_some_and(|i| i.show_requested()) {
            shared.log.write("second launch: show window");
            shared.show_by_user();
        }

        if Instant::now() >= next_check {
            next_check = Instant::now() + GAME_CHECK;
            match watcher.update(capture::is_game_running()) {
                Some(GameChange::Started) => {
                    shared.log.write("game started");
                    shared.set_game_running(true);
                    shared.reset_user_hidden();
                    reader.reset();
                    shared.set_reader(ReaderState::Starting);
                    shared.set_status_text("");
                }
                Some(GameChange::Exited) => {
                    shared.log.write("game exited");
                    shared.set_game_running(false);
                    reader.reset();
                    // Hidden through the gate: never while the user is typing in it.
                    gate.lost(now_ms());
                    shared.set_reader(ReaderState::Idle);
                    shared.set_status_text(WAITING_FOR_WOW);
                }
                None => {}
            }
        }

        if watcher.running() {
            match reader.tick() {
                Some(true) => {
                    if let Some(change) = gate.seen(now_ms()) {
                        shared.on_presence(change);
                    }
                }
                Some(false) => gate.lost(now_ms()),
                None => {}
            }
        }

        if let Some(change) = gate.poll(now_ms(), shared.hwnd().is_foreground()) {
            shared.on_presence(change);
        }

        std::thread::sleep(if watcher.running() { TICK } else { IDLE_TICK });
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
