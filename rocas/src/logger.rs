//! Dual-output logger: writes to stderr and an optional rotating log file.
//!
//! Implements the [`log::Log`] trait directly — no `env_logger` or other
//! logging framework required.
//!
//! # Format
//!
//! ```text
//! [2026-03-01T22:14:24Z INFO  rocas] message
//! ```
//!
//! # Level
//!
//! Filtering uses [`log::max_level`], so [`log::set_max_level`] changes the
//! verbosity of a running logger.
//!
//! # Colors
//!
//! ANSI colors are applied to the level tag when stderr is a terminal.
//! File output is always plain text.
//!
//! # Rotation
//!
//! When the active log file exceeds `max_size_bytes`, the logger rotates:
//!
//! ```text
//! rocas.log.3  (deleted)
//! rocas.log.2  → rocas.log.3
//! rocas.log.1  → rocas.log.2
//! rocas.log    → rocas.log.1
//! rocas.log    (new, empty)
//! ```
//!
//! With `keep_files = 0` the active file is truncated instead.

use std::ffi::OsString;
use std::fmt;
use std::fs::{File, OpenOptions, rename};
use std::io::{ErrorKind, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Error returned by [`Logger::init`].
#[derive(Debug, thiserror::Error)]
pub enum LoggerInitError {
    #[error("a global logger is already set: {0}")]
    AlreadySet(#[from] log::SetLoggerError),
    #[error("could not open log file: {0}")]
    Io(#[from] std::io::Error),
}

const RESET: &str = "\x1b[0m";

/// A Unix timestamp that displays as `YYYY-MM-DDTHH:MM:SSZ` (UTC, ISO 8601)
/// without any external crate and without allocating.
struct Timestamp(u64);

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0;
        let sec = secs % 60;
        let min = (secs / 60) % 60;
        let hour = (secs / 3_600) % 24;
        let days = secs / 86_400;

        // Gregorian calendar calculation (algorithm by Howard Hinnant).
        let shifted = days + 719_468;
        let era = shifted / 146_097;
        let doe = shifted - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = if month <= 2 { year + 1 } else { year };

        write!(f, "{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
    }
}

/// Formats a Unix timestamp as `YYYY-MM-DDTHH:MM:SSZ` (UTC, ISO 8601).
pub fn format_timestamp(secs: u64) -> String {
    Timestamp(secs).to_string()
}

/// Format a single log line the same way the logger writes to stderr —
/// including ANSI color on the level tag when stderr is a tty.
///
/// Useful for producing banner-side startup messages that look identical to
/// normal logger output.
pub fn format_line(ts: &str, level: log::Level, target: &str, msg: &str) -> String {
    if stderr_is_tty() {
        let color = level_color(level);
        format!("[{ts} {color}{level:<5}{RESET} {target}] {msg}")
    } else {
        format!("[{ts} {level:<5} {target}] {msg}")
    }
}

fn level_color(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "\x1b[31m", // red
        log::Level::Warn => "\x1b[33m",  // yellow
        log::Level::Info => "\x1b[36m",  // cyan
        log::Level::Debug => "\x1b[34m", // blue
        log::Level::Trace => "\x1b[2m",  // dimmed
    }
}

/// Returns `true` when stderr is connected to a terminal.
pub fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

/// Logger that writes to stderr (with optional ANSI colors) and an optional
/// rotating log file (always plain text).
pub struct Logger {
    /// Cached at init time — does not change while the process is running.
    use_color: bool,
    file: Option<Mutex<FileLogger>>,
}

impl Logger {
    /// Build and globally register the logger.
    ///
    /// `log_path` — `None` disables file logging (stderr only). `level` is
    /// installed as the global maximum level; change it later with
    /// [`log::set_max_level`].
    ///
    /// # Errors
    ///
    /// Returns [`log::SetLoggerError`] if a global logger is already
    /// registered, or [`std::io::Error`] if the log file cannot be opened
    /// or the parent directory cannot be created.
    pub fn init(
        level: log::LevelFilter,
        log_path: Option<PathBuf>,
        max_size_mb: u64,
        keep_files: u32,
    ) -> Result<(), LoggerInitError> {
        let file = log_path
            .map(|path| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                FileLogger::new(path, max_size_mb.saturating_mul(1024 * 1024), keep_files)
                    .map(Mutex::new)
            })
            .transpose()?;

        let logger = Box::new(Logger { use_color: stderr_is_tty(), file });
        log::set_boxed_logger(logger)?;
        log::set_max_level(level);
        Ok(())
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let ts = Timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        let level = record.level();
        let target = record.target();
        let args = record.args();

        // Formatting happens before any lock, and each sink gets the whole
        // line in a single write so concurrent lines never interleave.
        let plain = format!("[{ts} {level:<5} {target}] {args}\n");

        // A closed or full stderr must never take the daemon down.
        let _ = if self.use_color {
            let color = level_color(level);
            let colored = format!("[{ts} {color}{level:<5}{RESET} {target}] {args}\n");
            std::io::stderr().write_all(colored.as_bytes())
        } else {
            std::io::stderr().write_all(plain.as_bytes())
        };

        if let Some(mutex) = &self.file
            && let Ok(mut file) = mutex.lock()
            && let Err(e) = file.write_line(plain.as_bytes())
        {
            let _ = writeln!(std::io::stderr(), "[rocas logger] failed to write to log file: {e}");
        }
    }

    // The file is written unbuffered, one `write` per line: nothing is lost
    // if the process aborts, and there is nothing to flush.
    fn flush(&self) {}
}

struct FileLogger {
    file: File,
    path: PathBuf,
    current_size: u64,
    max_size_bytes: u64,
    keep_files: u32,
}

impl FileLogger {
    fn open(path: &Path, truncate: bool) -> std::io::Result<(File, u64)> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(!truncate)
            .truncate(truncate)
            .open(path)?;
        let size = file.metadata()?.len();
        Ok((file, size))
    }

    fn new(path: PathBuf, max_size_bytes: u64, keep_files: u32) -> std::io::Result<Self> {
        let (file, current_size) = Self::open(&path, false)?;
        Ok(Self {
            file,
            path,
            current_size,
            max_size_bytes,
            keep_files,
        })
    }

    /// `rocas.log` + 2 → `rocas.log.2`; appended to the full file name so a
    /// log path without the `.log` extension rotates next to itself.
    fn numbered(&self, index: u32) -> PathBuf {
        let mut name = OsString::from(self.path.as_os_str());
        name.push(format!(".{index}"));
        PathBuf::from(name)
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        if self.keep_files > 0 {
            for i in (1..self.keep_files).rev() {
                match rename(self.numbered(i), self.numbered(i + 1)) {
                    Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
                    _ => {},
                }
            }
            rename(&self.path, self.numbered(1))?;
        }

        let (file, size) = Self::open(&self.path, self.keep_files == 0)?;
        self.file = file;
        self.current_size = size;
        Ok(())
    }

    fn write_line(&mut self, line: &[u8]) -> std::io::Result<()> {
        if self.max_size_bytes > 0 && self.current_size >= self.max_size_bytes {
            self.rotate()?;
        }
        self.file.write_all(line)?;
        self.current_size += line.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rocas-logger-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        dir
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn timestamp_formats_the_epoch() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn timestamp_formats_a_leap_day() {
        assert_eq!(format_timestamp(1_709_210_096), "2024-02-29T12:34:56Z");
    }

    #[test]
    fn rotation_keeps_n_files_and_drops_the_oldest() {
        let dir = temp_dir("keep");
        let path = dir.join("rocas.log");
        let mut logger = FileLogger::new(path.clone(), 4, 2).expect("log file opens");

        for line in ["aaaa\n", "bbbb\n", "cccc\n", "dddd\n"] {
            logger
                .write_line(line.as_bytes())
                .expect("write succeeds");
        }

        assert_eq!(read(&path), "dddd\n");
        assert_eq!(read(&dir.join("rocas.log.1")), "cccc\n");
        assert_eq!(read(&dir.join("rocas.log.2")), "bbbb\n");
        assert!(!dir.join("rocas.log.3").exists());
    }

    #[test]
    fn rotation_without_a_log_extension_stays_next_to_the_file() {
        let dir = temp_dir("noext");
        let path = dir.join("rocas");
        let mut logger = FileLogger::new(path.clone(), 4, 1).expect("log file opens");

        logger
            .write_line(b"aaaa\n")
            .expect("write succeeds");
        logger
            .write_line(b"bbbb\n")
            .expect("write succeeds");

        assert_eq!(read(&path), "bbbb\n");
        assert_eq!(read(&dir.join("rocas.1")), "aaaa\n");
    }

    #[test]
    fn keeping_zero_files_truncates() {
        let dir = temp_dir("zero");
        let path = dir.join("rocas.log");
        let mut logger = FileLogger::new(path.clone(), 4, 0).expect("log file opens");

        logger
            .write_line(b"aaaa\n")
            .expect("write succeeds");
        logger
            .write_line(b"bbbb\n")
            .expect("write succeeds");

        assert_eq!(read(&path), "bbbb\n");
        assert!(!dir.join("rocas.log.1").exists());
    }

    #[test]
    fn zero_max_size_never_rotates() {
        let dir = temp_dir("norot");
        let path = dir.join("rocas.log");
        let mut logger = FileLogger::new(path.clone(), 0, 3).expect("log file opens");

        for _ in 0..3 {
            logger
                .write_line(b"aaaa\n")
                .expect("write succeeds");
        }

        assert_eq!(read(&path), "aaaa\naaaa\naaaa\n");
        assert!(!dir.join("rocas.log.1").exists());
    }

    #[test]
    fn reopening_appends_and_counts_existing_bytes() {
        let dir = temp_dir("reopen");
        let path = dir.join("rocas.log");
        std::fs::write(&path, "old\n").expect("seed file is writable");
        let mut logger = FileLogger::new(path.clone(), 8, 1).expect("log file opens");

        logger
            .write_line(b"aaaa\n")
            .expect("write succeeds");
        logger
            .write_line(b"bbbb\n")
            .expect("write succeeds");

        assert_eq!(read(&dir.join("rocas.log.1")), "old\naaaa\n");
        assert_eq!(read(&path), "bbbb\n");
    }
}
