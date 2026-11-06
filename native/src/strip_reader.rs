//! Finds and decodes the addon's pixel strip inside a captured RGB patch taken from the
//! top-left corner of the WoW client area. Port of `app/src/core/strip-reader.js`: the
//! cell size (1-4 physical px) and a small offset (0-2 px) are discovered automatically;
//! the layout that worked last time is tried first.

use crate::strip::{decode_frame, Frame, COLS, ROWS};

const CELL_SIZES: [usize; 4] = [2, 3, 4, 1];
const OFFSETS: [usize; 3] = [0, 1, 2];

/// Where the strip was found inside the patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub cell: usize,
    pub dx: usize,
    pub dy: usize,
}

fn sample(px: &[u8], w: usize, h: usize, l: Layout) -> Option<Vec<[u8; 3]>> {
    let half = l.cell / 2;
    let mut colors = Vec::with_capacity(COLS * ROWS);
    for i in 0..COLS * ROWS {
        let x = l.dx + (i % COLS) * l.cell + half;
        let y = l.dy + (i / COLS) * l.cell + half;
        if x >= w || y >= h {
            return None;
        }
        let o = (y * w + x) * 3;
        colors.push([*px.get(o)?, *px.get(o + 1)?, *px.get(o + 2)?]);
    }
    Some(colors)
}

/// Cheap test before a full decode: the first marker cell is strong red + some blue,
/// with no green (levels 15, 0, 8).
fn looks_like_marker(px: &[u8], w: usize, l: Layout) -> bool {
    let half = l.cell / 2;
    let o = ((l.dy + half) * w + l.dx + half) * 3;
    matches!((px.get(o), px.get(o + 1)), (Some(&r), Some(&g)) if r > 150 && g < 100)
}

/// `px`: w*h tightly packed RGB triplets. `hint` remembers the working layout.
/// Returns the frame (message `None` for the idle frame) or `None` when no strip.
pub fn find_strip(px: &[u8], w: usize, h: usize, hint: &mut Option<Layout>) -> Option<Frame> {
    let mut tries: Vec<Layout> = Vec::with_capacity(37);
    tries.extend(*hint);
    for cell in CELL_SIZES {
        for dy in OFFSETS {
            for dx in OFFSETS {
                tries.push(Layout { cell, dx, dy });
            }
        }
    }
    for l in tries {
        if l.dx + COLS * l.cell > w || l.dy + ROWS * l.cell > h {
            continue;
        }
        if !looks_like_marker(px, w, l) {
            continue;
        }
        if let Some(frame) = sample(px, w, h, l).and_then(|c| decode_frame(&c)) {
            *hint = Some(l);
            return Some(frame);
        }
    }
    None
}
