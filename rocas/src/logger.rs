//! Dual-output logger: writes to stderr and an optional rotating log file.
//!
//! Implements the [`log::Log`] trait directly — no `env_logger` or other
//! logging framework required.
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

/// A single log record written to both stderr and a file.
pub struct Logger {
    level: log::LevelFilter,
    file: Option<Mutex<FileLogger>>,
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
        // Flush before rotating.
        self.writer.flush()?;

        // Shift old files: rocas.log.N-1 → rocas.log.N, dropping the oldest.
        for i in (1..self.keep_files).rev() {
            let from = self
                .path
                .with_extension(format!("log.{i}"));
            let to = self
                .path
                .with_extension(format!("log.{}", i + 1));
            if from.exists() {
                // Ignore errors (e.g. permissions) — best-effort rotation.
                let _ = rename(&from, &to);
            }
        }

        // rocas.log → rocas.log.1
        if self.keep_files > 0 {
            let _ = rename(&self.path, self.path.with_extension("log.1"));
        }

        // Open fresh file.
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
        let file = log_path
            .map(|path| {
                // Create parent directories if needed.
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                FileLogger::new(path, max_size_mb * 1024 * 1024, keep_files)
                    .map(Mutex::new)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
            .transpose()?;

        let logger = Box::new(Logger { level, file });
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

        let now = {
            // Minimal timestamp without pulling in chrono — seconds since epoch
            // formatted as a UTC-ish string. Good enough for log files.
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format_timestamp(secs)
        };

        let line = format!("{} [{:>5}] {}\n", now, record.level(), record.args());

        // Always write to stderr.
        eprint!("{line}");

        // Write to file if configured.
        if let Some(mutex) = &self.file
            && let Ok(mut file_logger) = mutex.lock()
            && let Err(e) = file_logger.write_line(&line)
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

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM:SS` (UTC) without any
/// external crate.
fn format_timestamp(secs: u64) -> String {
    // Days since Unix epoch broken into date components.
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

    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}
