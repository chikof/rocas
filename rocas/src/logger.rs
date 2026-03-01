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
//! # Colors
//!
//! ANSI colors are applied to the level tag when stderr is a terminal.
//! File output is always plain text. On Windows, `SetConsoleMode` is called
//! at init time to opt in to virtual terminal processing.
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

use std::fs::{File, OpenOptions, rename};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const RESET: &str = "\x1b[0m";

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
    #[cfg(unix)]
    {
        // SAFETY: isatty is always safe to call with a valid fd.
        unsafe { libc::isatty(libc::STDERR_FILENO) != 0 }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;

        use windows_sys::Win32::System::Console::GetConsoleMode;
        let handle = std::io::stderr().as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        let mut mode: u32 = 0;
        // GetConsoleMode succeeds only when the handle is a real console,
        // not a pipe or redirected file.
        unsafe { GetConsoleMode(handle, &mut mode) != 0 }
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// On Windows 10+, enable virtual terminal processing so ANSI escape codes
/// render in the console. No-op on other platforms.
fn enable_ansi_on_windows() {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;

        use windows_sys::Win32::System::Console::{
            ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            GetConsoleMode,
            SetConsoleMode,
        };
        let handle = std::io::stderr().as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        let mut mode: u32 = 0;
        unsafe {
            if GetConsoleMode(handle, &mut mode) != 0 {
                let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }
}

/// Logger that writes to stderr (with optional ANSI colors) and an optional
/// rotating log file (always plain text).
pub struct Logger {
    level: log::LevelFilter,
    /// Cached at init time — does not change while the process is running.
    use_color: bool,
    file: Option<Mutex<FileLogger>>,
}

impl Logger {
    /// Build and globally register the logger.
    ///
    /// `log_path` — `None` disables file logging (stderr only).
    pub fn init(
        level: log::LevelFilter,
        log_path: Option<PathBuf>,
        max_size_mb: u64,
        keep_files: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        enable_ansi_on_windows();
        let use_color = stderr_is_tty();

        let file = log_path
            .map(|path| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                FileLogger::new(path, max_size_mb * 1024 * 1024, keep_files)
                    .map(Mutex::new)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
            .transpose()?;

        let logger = Box::new(Logger { level, use_color, file });
        log::set_boxed_logger(logger)?;
        log::set_max_level(level);
        Ok(())
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ts = format_timestamp(secs);

        let level = record.level();
        let target = record.target();
        let args = record.args();

        // Plain line written to the log file (no ANSI codes).
        let plain = format!("[{ts} {level:<5} {target}] {args}\n");

        // Stderr: color the level tag if we're on a tty.
        if self.use_color {
            let color = level_color(level);
            eprintln!("[{ts} {color}{level:<5}{RESET} {target}] {args}");
        } else {
            eprint!("{plain}");
        }

        // Write plain text to file.
        if let Some(mutex) = &self.file
            && let Ok(mut fl) = mutex.lock()
            && let Err(e) = fl.write_line(&plain)
        {
            eprintln!("[rocas logger] failed to write to log file: {e}");
        }
    }

    fn flush(&self) {
        if let Some(mutex) = &self.file
            && let Ok(mut fl) = mutex.lock()
        {
            let _ = fl.writer.flush();
        }
    }
}

struct FileLogger {
    writer: BufWriter<File>,
    path: PathBuf,
    current_size: u64,
    max_size_bytes: u64,
    keep_files: u32,
}

impl FileLogger {
    fn open(path: &Path) -> std::io::Result<(BufWriter<File>, u64)> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let size = file.metadata()?.len();
        Ok((BufWriter::new(file), size))
    }

    fn new(path: PathBuf, max_size_bytes: u64, keep_files: u32) -> std::io::Result<Self> {
        let (writer, current_size) = Self::open(&path)?;
        Ok(Self {
            writer,
            path,
            current_size,
            max_size_bytes,
            keep_files,
        })
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        self.writer.flush()?;

        for i in (1..self.keep_files).rev() {
            let from = self
                .path
                .with_extension(format!("log.{i}"));
            let to = self
                .path
                .with_extension(format!("log.{}", i + 1));
            if from.exists() {
                let _ = rename(&from, &to);
            }
        }

        if self.keep_files > 0 {
            let _ = rename(&self.path, self.path.with_extension("log.1"));
        }

        let (writer, _) = Self::open(&self.path)?;
        self.writer = writer;
        self.current_size = 0;
        Ok(())
    }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.max_size_bytes > 0 && self.current_size >= self.max_size_bytes {
            self.rotate()?;
        }
        let bytes = line.as_bytes();
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        self.current_size += bytes.len() as u64;
        Ok(())
    }
}

/// Format a Unix timestamp as `YYYY-MM-DDTHH:MM:SSZ` (UTC ISO 8601) without
/// any external crate.
pub fn format_timestamp(secs: u64) -> String {
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;

    // Gregorian calendar calculation.
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };

    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}
