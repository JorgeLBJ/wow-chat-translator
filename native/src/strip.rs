//! Pixel-strip protocol: how the addon sends a chat message as a small grid of colored
//! cells, and how the app decodes it exactly (no OCR).
//!
//! Exact port of `app/src/core/strip.js`; the Lua encoder in the addon produces what
//! [`encode_frame`] produces.
//!
//! Layout (COLS x ROWS cells, read row by row):
//!   cells 0-3    marker
//!   cells 4-19   calibration ramp, cell 4+i = (i, i, i)
//!   cell  20     sequence number (12 bits)
//!   cell  21     payload length in bytes (12 bits)
//!   cells 22-23  checksum: two 12-bit running sums mod 4095
//!   cells 24+    payload bytes, as 4-bit nibbles (high first), 3 nibbles per cell
//! Every cell carries three 4-bit levels (R, G, B), drawn on screen as level * 17.

pub const COLS: usize = 64;
pub const ROWS: usize = 6;
pub const CELLS: usize = COLS * ROWS;
pub const MARKER: [[u8; 3]; 4] = [[15, 0, 8], [0, 15, 15], [15, 8, 0], [3, 0, 15]];
const CAL: usize = 4;
const SEQ: usize = 20;
const LEN: usize = 21;
const SUM: usize = 22;
const DATA: usize = 24;
pub const MAX_PAYLOAD: usize = (CELLS - DATA) * 3 / 2;
const SEP: char = '\x1F';
const VERSION: &str = "v1";

/// A decoded chat message. `class` is `None` when the addon did not know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub channel: String,
    pub class: Option<String>,
    pub player: String,
    pub text: String,
}

/// A valid frame. `message` is `None` for the idle frame (addon alive, nothing to send).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub seq: u16,
    pub message: Option<Message>,
}

fn to12(v: u32) -> [u8; 3] {
    [((v >> 8) & 15) as u8, ((v >> 4) & 15) as u8, (v & 15) as u8]
}

fn from12(c: [u8; 3]) -> u32 {
    ((c[0] as u32) << 8) | ((c[1] as u32) << 4) | c[2] as u32
}

fn checksum(bytes: &[u8]) -> [u32; 2] {
    let (mut s1, mut s2) = (0u32, 0u32);
    for &b in bytes {
        s1 = (s1 + b as u32) % 4095;
        s2 = (s2 + s1) % 4095;
    }
    [s1, s2]
}

fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Least-squares fit y = a + b*x + c*x^2 over x = 0..15; returns the fitted 16 values.
fn fit_quadratic(ys: &[f64; 16]) -> [f64; 16] {
    let (mut s0, mut s1, mut s2, mut s3, mut s4) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut t0, mut t1, mut t2) = (0.0, 0.0, 0.0);
    for (x, &y) in ys.iter().enumerate() {
        let x = x as f64;
        let x2 = x * x;
        s0 += 1.0;
        s1 += x;
        s2 += x2;
        s3 += x2 * x;
        s4 += x2 * x2;
        t0 += y;
        t1 += x * y;
        t2 += x2 * y;
    }
    // Solve the 3x3 normal equations with Cramer's rule.
    let m = [[s0, s1, s2], [s1, s2, s3], [s2, s3, s4]];
    let t = [t0, t1, t2];
    let d = det3(&m);
    let col = |k: usize| {
        let mut mk = m;
        for r in 0..3 {
            mk[r][k] = t[r];
        }
        det3(&mk) / d
    };
    let (a, b, c) = (col(0), col(1), col(2));
    let mut out = [0.0; 16];
    for (x, v) in out.iter_mut().enumerate() {
        let x = x as f64;
        *v = a + b * x + c * x * x;
    }
    out
}

// ---- payload -------------------------------------------------------------------

/// `v1 SEP channel SEP class SEP player SEP text`, UTF-8. Text that would not fit is
/// cut by whole characters so the UTF-8 stays valid.
pub fn build_payload(m: &Message) -> Vec<u8> {
    let sep = SEP.to_string();
    let head = [VERSION, &m.channel, m.class.as_deref().unwrap_or(""), &m.player, ""].join(&sep);
    let mut out = head.into_bytes();
    let budget = MAX_PAYLOAD.saturating_sub(out.len());
    let mut cut = 0;
    for (i, ch) in m.text.char_indices() {
        if i + ch.len_utf8() > budget {
            break;
        }
        cut = i + ch.len_utf8();
    }
    out.extend_from_slice(&m.text.as_bytes()[..cut]);
    out
}

/// Inverse of [`build_payload`]; `None` for invalid UTF-8, a wrong version or fewer
/// than five fields. The text may itself contain the separator.
pub fn parse_payload(bytes: &[u8]) -> Option<Message> {
    let s = std::str::from_utf8(bytes).ok()?;
    let parts: Vec<&str> = s.splitn(5, SEP).collect();
    if parts.len() < 5 || parts[0] != VERSION {
        return None;
    }
    Some(Message {
        channel: parts[1].to_string(),
        class: (!parts[2].is_empty()).then(|| parts[2].to_string()),
        player: parts[3].to_string(),
        text: parts[4].to_string(),
    })
}

// ---- frames --------------------------------------------------------------------

/// Returns CELLS cells of [r, g, b] levels (0-15).
///
/// # Panics
/// When `payload` is longer than [`MAX_PAYLOAD`] (the JS encoder throws).
pub fn encode_frame(seq: u32, payload: &[u8]) -> Vec<[u8; 3]> {
    assert!(payload.len() <= MAX_PAYLOAD, "payload too long");
    let mut cells = vec![[0u8; 3]; CELLS];
    cells[..MARKER.len()].copy_from_slice(&MARKER);
    for i in 0..16u8 {
        cells[CAL + i as usize] = [i, i, i];
    }
    cells[SEQ] = to12(seq % 4096);
    cells[LEN] = to12(payload.len() as u32);
    let [s1, s2] = checksum(payload);
    cells[SUM] = to12(s1);
    cells[SUM + 1] = to12(s2);
    let nibbles: Vec<u8> = payload.iter().flat_map(|&b| [b >> 4, b & 15]).collect();
    for (k, chunk) in nibbles.chunks(3).enumerate() {
        let mut c = [0u8; 3];
        c[..chunk.len()].copy_from_slice(chunk);
        cells[DATA + k] = c;
    }
    cells
}

/// `colors`: CELLS samples of [r, g, b] (0-255) taken at each cell center.
/// Returns `None` when the area is not a valid frame.
pub fn decode_frame(colors: &[[u8; 3]]) -> Option<Frame> {
    if colors.len() < CELLS {
        return None;
    }
    // Learn how each of the 16 levels looks on this screen, per channel. A quadratic
    // fit over the 16 calibration samples smooths their noise and still follows a
    // gamma-shifted or darker screen.
    let mut reference = [[0.0f64; 16]; 3];
    for (ch, r) in reference.iter_mut().enumerate() {
        let mut raw = [0.0f64; 16];
        for (i, v) in raw.iter_mut().enumerate() {
            *v = colors[CAL + i][ch] as f64;
        }
        if raw[15] - raw[0] < 60.0 {
            return None;
        }
        *r = fit_quadratic(&raw);
    }
    let level = |v: u8, ch: usize| -> u8 {
        let v = v as f64;
        let r = &reference[ch];
        let mut best = 0;
        for i in 1..16 {
            if (v - r[i]).abs() < (v - r[best]).abs() {
                best = i;
            }
        }
        best as u8
    };
    let cell = |i: usize| -> [u8; 3] {
        let c = colors[i];
        [level(c[0], 0), level(c[1], 1), level(c[2], 2)]
    };

    for (i, m) in MARKER.iter().enumerate() {
        if cell(i) != *m {
            return None;
        }
    }
    let seq = from12(cell(SEQ)) as u16;
    let length = from12(cell(LEN)) as usize;
    if length > MAX_PAYLOAD {
        return None;
    }
    let expected = [from12(cell(SUM)), from12(cell(SUM + 1))];

    let mut nibbles = Vec::with_capacity(length * 2 + 2);
    let mut i = DATA;
    while nibbles.len() < length * 2 {
        nibbles.extend_from_slice(&cell(i));
        i += 1;
    }
    let bytes: Vec<u8> = (0..length)
        .map(|k| (nibbles[2 * k] << 4) | nibbles[2 * k + 1])
        .collect();

    if checksum(&bytes) != expected {
        return None;
    }
    // Empty payload = idle frame: the addon is running but has no message to send.
    if length == 0 {
        return Some(Frame { seq, message: None });
    }
    parse_payload(&bytes).map(|m| Frame { seq, message: Some(m) })
}
