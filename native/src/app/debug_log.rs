//! Diagnostic log (`debug.log` next to the config): strip reads, presence changes and
//! what the overlay received, so a "stuck" report can be checked against evidence.
//! Rotated to `debug.log.old` past the size limit (2 MB in the app, like Electron).
//! Logging never fails the caller.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEBUG_MAX_BYTES: u64 = 2 * 1024 * 1024;

pub struct DebugLog {
    path: PathBuf,
    max_bytes: u64,
    lock: Mutex<()>,
}

impl DebugLog {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self { path, max_bytes, lock: Mutex::new(()) }
    }

    /// Appends one timestamped line.
    pub fn write(&self, line: &str) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.append(line);
    }

    fn append(&self, line: &str) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if std::fs::metadata(&self.path).map(|m| m.len() > self.max_bytes).unwrap_or(false) {
            let mut old = self.path.clone().into_os_string();
            old.push(".old");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&self.path, &old)?;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(file, "{} {line}", iso_utc(now))
    }
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for ms since the Unix epoch (like JS `toISOString`).
pub fn iso_utc(epoch_ms: u64) -> String {
    let secs = epoch_ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        epoch_ms % 1000
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}
