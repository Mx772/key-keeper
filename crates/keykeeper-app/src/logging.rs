//! The activity log: what network work did (requests, rate limits, failures, results), for
//! Settings → Diagnostics and for bug reports. Written to `keykeeper.log` in the app's data
//! folder (rotated at [`MAX_BYTES`], one old file kept) and the last [`RECENT`] lines kept in
//! memory. Never holds keys or API keys: the Steam client logs only host and path, and every
//! line goes through [`keykeeper_steam::http::redact`] as well.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// The log file is moved to `keykeeper.log.1` past this size.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Lines kept in memory for the Settings view.
pub const RECENT: usize = 400;
pub const FILE_NAME: &str = "keykeeper.log";

struct Sink {
    path: Option<PathBuf>,
    file: Option<File>,
    written: u64,
    recent: VecDeque<String>,
}

struct Logger {
    sink: Mutex<Sink>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

fn ours(target: &str) -> bool {
    target.starts_with("keykeeper")
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        // Our crates in detail; libraries (GPU, windowing) only when something is wrong.
        if ours(m.target()) {
            m.level() <= Level::Debug
        } else {
            m.level() <= Level::Warn
        }
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let now = jiff::Timestamp::now()
            .to_zoned_datetime(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M:%S");
        let target = r.target().strip_prefix("keykeeper_").unwrap_or(r.target());
        let target = target.split("::").next().unwrap_or(target);
        let msg = keykeeper_steam::http::redact(&r.args().to_string());
        let line = format!("{now} {:<5} {target}: {msg}", r.level());
        if let Ok(mut s) = self.sink.lock() {
            s.write(&line);
        }
    }

    fn flush(&self) {
        if let Ok(mut s) = self.sink.lock()
            && let Some(f) = s.file.as_mut()
        {
            let _ = f.flush();
        }
    }
}

impl Sink {
    fn write(&mut self, line: &str) {
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(line.to_string());
        if self.written > MAX_BYTES {
            self.rotate();
        }
        if let Some(f) = self.file.as_mut()
            && writeln!(f, "{line}").is_ok()
        {
            self.written += line.len() as u64 + 1;
        }
    }

    fn rotate(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.file = None;
        let _ = std::fs::rename(&path, old_path(&path));
        self.file = open(&path);
        self.written = 0;
    }
}

fn open(path: &Path) -> Option<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn old_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".1");
    PathBuf::from(p)
}

/// Start logging to `dir/keykeeper.log` (memory only when `dir` is `None` or unwritable).
/// Later calls do nothing.
pub fn init(dir: Option<PathBuf>) {
    let path = dir.map(|d| d.join(FILE_NAME));
    let file = path.as_deref().and_then(open);
    let written = file
        .as_ref()
        .and_then(|f| f.metadata().ok())
        .map_or(0, |m| m.len());
    let logger = LOGGER.get_or_init(|| Logger {
        sink: Mutex::new(Sink {
            path,
            file,
            written,
            recent: VecDeque::new(),
        }),
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Debug);
        log::info!(
            "KeyKeeper {} started ({} {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
}

/// The log file, when logging to one.
pub fn path() -> Option<PathBuf> {
    LOGGER.get()?.sink.lock().ok()?.path.clone()
}

/// The most recent lines, oldest first.
pub fn recent(n: usize) -> Vec<String> {
    let Some(l) = LOGGER.get() else {
        return Vec::new();
    };
    let Ok(s) = l.sink.lock() else {
        return Vec::new();
    };
    let skip = s.recent.len().saturating_sub(n);
    s.recent.iter().skip(skip).cloned().collect()
}

/// The whole log (the rotated-out file first), for exporting. Falls back to the lines in memory.
pub fn full_text() -> String {
    log::logger().flush();
    if let Some(p) = path() {
        let mut out = std::fs::read_to_string(old_path(&p)).unwrap_or_default();
        if let Ok(cur) = std::fs::read_to_string(&p) {
            out.push_str(&cur);
            return out;
        }
    }
    let mut out = recent(RECENT).join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_keeps_recent_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut s = Sink {
            path: Some(path.clone()),
            file: open(&path),
            written: MAX_BYTES + 1,
            recent: VecDeque::new(),
        };
        s.write("first");
        assert!(old_path(&path).exists(), "rotated before writing");
        for i in 0..RECENT + 5 {
            s.write(&format!("line {i}"));
        }
        assert_eq!(s.recent.len(), RECENT);
        assert_eq!(s.recent.back().unwrap(), &format!("line {}", RECENT + 4));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("first\n"));
    }
}
