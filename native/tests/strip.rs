// Ported from app/test/strip.test.js, plus a cross-language golden frame produced by
// the JS encoder (tests/golden_frame.json).

use wow_chat_translator::strip::{
    build_payload, decode_frame, encode_frame, parse_payload, Frame, Message, CELLS, MAX_PAYLOAD,
};
use wow_chat_translator::strip_reader::{find_strip, Layout};

fn msg() -> Message {
    Message {
        channel: "Trade".into(),
        class: Some("PALADIN".into()),
        player: "La Muertita".into(),
        text: "WTS [Blacksmithing: Runed Copper Bracers] 5g, lol ñandú 😀".into(),
    }
}

/// What the screen shows: each level drawn as level * 17, then transformed by `f`.
fn to_colors(levels: &[[u8; 3]], mut f: impl FnMut(i32) -> i32) -> Vec<[u8; 3]> {
    levels
        .iter()
        .map(|c| {
            let r = f(c[0] as i32 * 17);
            let g = f(c[1] as i32 * 17);
            let b = f(c[2] as i32 * 17);
            [r as u8, g as u8, b as u8]
        })
        .collect()
}

fn plain(levels: &[[u8; 3]]) -> Vec<[u8; 3]> {
    to_colors(levels, |v| v)
}

#[test]
fn round_trips_links_accents_and_emoji() {
    let frame = encode_frame(7, &build_payload(&msg()));
    assert_eq!(frame.len(), CELLS);
    assert_eq!(decode_frame(&plain(&frame)), Some(Frame { seq: 7, message: Some(msg()) }));
}

#[test]
fn survives_color_noise() {
    let mut i: i32 = 0;
    let noise = |v: i32| {
        let n = (v + ((i * 37) % 13) - 6).clamp(0, 255);
        i += 1;
        n
    };
    let colors = to_colors(&encode_frame(3, &build_payload(&msg())), noise);
    assert_eq!(decode_frame(&colors).and_then(|f| f.message), Some(msg()));
}

#[test]
fn survives_darker_gamma_shifted_screen() {
    let dark = |v: i32| (255.0 * (v as f64 / 255.0).powf(1.25) * 0.85).round() as i32;
    let colors = to_colors(&encode_frame(9, &build_payload(&msg())), dark);
    assert_eq!(decode_frame(&colors).and_then(|f| f.message), Some(msg()));
}

#[test]
fn rejects_non_strip_scenery() {
    let scenery: Vec<[u8; 3]> = (0..CELLS)
        .map(|k| [((k * 7) % 255) as u8, ((k * 13) % 255) as u8, ((k * 29) % 255) as u8])
        .collect();
    assert_eq!(decode_frame(&scenery), None);
}

#[test]
fn rejects_corrupted_payload() {
    let mut frame = encode_frame(1, &build_payload(&msg()));
    frame[30][0] = (frame[30][0] + 5) % 16;
    assert_eq!(decode_frame(&plain(&frame)), None);
}

#[test]
fn rejects_too_few_cells() {
    let frame = encode_frame(1, &build_payload(&msg()));
    assert_eq!(decode_frame(&plain(&frame[..CELLS - 1])), None);
}

#[test]
fn wraps_sequence_at_12_bits() {
    let frame = encode_frame(4096 + 5, &build_payload(&msg()));
    assert_eq!(decode_frame(&plain(&frame)).map(|f| f.seq), Some(5));
}

#[test]
fn truncates_without_breaking_utf8() {
    let long = Message { text: "á".repeat(400), ..msg() };
    let bytes = build_payload(&long);
    assert!(bytes.len() <= MAX_PAYLOAD);
    let back = parse_payload(&bytes).expect("valid payload");
    assert!(back.text.chars().count() > 200);
    assert!(back.text.chars().all(|c| c == 'á'));
}

#[test]
fn keeps_unknown_class_empty() {
    let back = parse_payload(&build_payload(&Message { class: None, ..msg() })).unwrap();
    assert_eq!(back.class, None);
}

#[test]
fn text_may_contain_separator() {
    let m = Message { text: "a\x1Fb".into(), ..msg() };
    assert_eq!(parse_payload(&build_payload(&m)), Some(m));
}

#[test]
fn idle_frame_has_no_message() {
    let frame = encode_frame(42, &[]);
    assert_eq!(decode_frame(&plain(&frame)), Some(Frame { seq: 42, message: None }));
}

#[test]
fn matches_js_golden_frame() {
    let golden: Vec<[u8; 3]> =
        serde_json::from_str(include_str!("golden_frame.json")).expect("golden json");
    assert_eq!(golden.len(), CELLS);
    let frame = encode_frame(1234, &build_payload(&msg()));
    assert_eq!(frame, golden);
}

// ---- find_strip ----------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Paint {
    cell: usize,
    dx: usize,
    dy: usize,
    edge_blur: bool,
}

const W: usize = 260;
const H: usize = 28;

/// Draws a frame into an RGB patch the way WoW would (see paint() in strip.test.js).
fn paint(frame: &[[u8; 3]], o: Paint) -> Vec<u8> {
    let mut px: Vec<u8> = (0..W * H * 3).map(|i| ((i * 31) % 200) as u8).collect();
    for (i, c) in frame.iter().enumerate() {
        let cx = (i % 64) * o.cell + o.dx;
        let cy = (i / 64) * o.cell + o.dy;
        for y in 0..o.cell {
            for x in 0..o.cell {
                let off = ((cy + y) * W + (cx + x)) * 3;
                let blur = if o.edge_blur && (x == 0 || y == 0) { 0.5 } else { 1.0 };
                for ch in 0..3 {
                    let v = c[ch] as f64 * 17.0 * blur + px[off + ch] as f64 * (1.0 - blur);
                    px[off + ch] = v as u8; // truncates like a Uint8Array store
                }
            }
        }
    }
    px
}

fn find_cases() -> Vec<Paint> {
    vec![
        Paint { cell: 2, ..Default::default() },
        Paint { cell: 3, dx: 1, ..Default::default() },
        Paint { cell: 4, dy: 2, ..Default::default() },
        Paint { cell: 1, dx: 2, dy: 1, ..Default::default() },
        Paint { cell: 3, edge_blur: true, ..Default::default() },
    ]
}

#[test]
fn find_strip_finds_and_decodes() {
    let frame = encode_frame(77, &build_payload(&msg()));
    for o in find_cases() {
        let px = paint(&frame, o);
        let got = find_strip(&px, W, H, &mut None).and_then(|f| f.message);
        assert_eq!(got, Some(msg()), "cell={} dx={} dy={} blur={}", o.cell, o.dx, o.dy, o.edge_blur);
    }
}

#[test]
fn find_strip_none_without_strip() {
    let px: Vec<u8> = (0..W * H * 3).map(|i| ((i * 13) % 251) as u8).collect();
    assert_eq!(find_strip(&px, W, H, &mut None), None);
}

#[test]
fn find_strip_remembers_layout() {
    let frame = encode_frame(77, &build_payload(&msg()));
    let px = paint(&frame, Paint { cell: 3, dx: 1, ..Default::default() });
    let mut hint = None;
    find_strip(&px, W, H, &mut hint);
    // Like the JS test, only the cell size is pinned: with cell 3, dx 0 also samples
    // inside every cell, and it is tried before dx 1.
    assert_eq!(hint.map(|l: Layout| l.cell), Some(3));
    assert_eq!(find_strip(&px, W, H, &mut hint).and_then(|f| f.message), Some(msg()));
}

/// Every WoW client counts as "the game", not only `Wow.exe` (Classic, PTR, Beta),
/// while helpers and this app (which also starts with "wow") do not.
#[cfg(windows)]
#[test]
fn game_process_names_cover_every_flavor() {
    use wow_chat_translator::capture::is_wow_client_exe;
    for exe in ["Wow.exe", "WOW.EXE", "WowClassic.exe", "WowClassicT.exe", "WowClassicB.exe", "WowT.exe", "WowB.exe"] {
        assert!(is_wow_client_exe(exe), "{exe}");
    }
    for exe in ["WowVoiceProxy.exe", "wow-chat-translator.exe", "WowError.exe", "Battle.net.exe"] {
        assert!(!is_wow_client_exe(exe), "{exe}");
    }
}
