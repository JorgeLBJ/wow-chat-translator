//! Debug tool: captures the WoW strip every 35 ms and prints each new decoded frame.
//!
//! Usage: strip-debug [--seconds N]

#[cfg(not(windows))]
fn main() {
    eprintln!("strip-debug only runs on Windows");
}

#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};
    use wow_chat_translator::capture::{self, Capturer, Grab, PATCH_H, PATCH_W};
    use wow_chat_translator::strip_reader::find_strip;

    let args: Vec<String> = std::env::args().collect();
    let limit = args
        .iter()
        .position(|a| a == "--seconds")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs);

    capture::init_dpi_awareness();
    println!("wow process running: {}", capture::is_game_running());

    let start = Instant::now();
    let mut capturer = Capturer::new();
    let mut hint = None;
    let mut last_hash: Option<u64> = None;
    let mut last_seq: Option<u16> = None;
    let mut state = "";

    // Prints a state line only when that state starts.
    let mut enter = |s: &'static str, line: &str| {
        if state != s {
            println!("{line}");
            state = s;
        }
    };

    while limit.map_or(true, |l| start.elapsed() < l) {
        match capturer.grab() {
            Ok(Grab::WowNotFound) => {
                last_hash = None;
                enter("nowow", "WoW not found");
            }
            Ok(Grab::Minimized) => {
                last_hash = None;
                enter("minimized", "WoW minimized");
            }
            Ok(Grab::Patch(px)) => {
                let hash = fnv1a(&px);
                if last_hash != Some(hash) {
                    last_hash = Some(hash);
                    match find_strip(&px, PATCH_W, PATCH_H, &mut hint) {
                        None => enter("nostrip", "no strip visible"),
                        Some(frame) => {
                            enter("strip", "strip visible");
                            if last_seq != Some(frame.seq) {
                                last_seq = Some(frame.seq);
                                let cell = hint.map_or(0, |l| l.cell);
                                match frame.message {
                                    None => println!("seq={} idle", frame.seq),
                                    Some(m) => println!(
                                        "seq={} cell={} channel={} class={} player={} text={}",
                                        frame.seq,
                                        cell,
                                        m.channel,
                                        m.class.as_deref().unwrap_or("-"),
                                        m.player,
                                        m.text
                                    ),
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => println!("error: {e}"),
        }
        std::thread::sleep(Duration::from_millis(35));
    }
}

#[cfg(windows)]
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
