#![windows_subsystem = "windows"]

use std::path::Path;
use std::time::{Duration, Instant};

use config::{Config, RuleConfig};
use pattern::Pattern;
use watcher::{DirWatcher, FileEvent, WatcherConfig};

mod art;
mod config;
mod logger;
mod pattern;
mod update;

#[macro_use]
extern crate log;

/// How often to probe file size while waiting for a download to finish.
const STABLE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum time to wait for a file to stop growing before moving it anyway.
const STABLE_MAX_WAIT: Duration = Duration::from_secs(300);

/// All errors that can occur in the rocas binary.
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("logger initialisation failed: {0}")]
    Logger(#[from] logger::LoggerInitError),

    #[error("watcher error: {0}")]
    Watcher(#[from] watcher::Error),

    #[error("auto-launch error: {0}")]
    AutoLaunch(#[from] auto_launch::Error),

    #[cfg(feature = "self-update")]
    #[error("update check failed: {0}")]
    Update(#[from] self_update::errors::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "self-update")]
    #[error("failed to restart process: {0}")]
    Restart(String),

    #[error("{0}")]
    Other(String),
}

fn main() -> Result<(), AppError> {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args
        .iter()
        .any(|a| a == "--version" || a == "-V")
    {
        println!("rocas {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // `--boot` is an action, not a setting, so it never reaches the config
    // layers: a file or env value would toggle autostart on every launch.
    let len = args.len();
    args.retain(|a| a != "--boot" && a != "-b");
    let boot = args.len() != len;

    let config = Config::load(args);

    if boot {
        config::startup_toggle()?;
    }

    // Resolve the log file path: explicit config value, or the OS data dir.
    let log_path = config
        .misc
        .log_file
        .as_deref()
        .map(std::path::PathBuf::from);

    logger::Logger::init(
        config.misc.log_level(),
        log_path,
        config.misc.log_max_size_mb,
        config.misc.log_keep_files,
    )?;

    run(&config)
}

fn run(config: &Config) -> Result<(), AppError> {
    if config.misc.check_for_updates {
        update::check(config.misc.auto_update)?;
    }

    let compiled_rules: Vec<(Vec<Pattern>, &RuleConfig)> = config
        .rules
        .iter()
        .map(|r| (r.compiled_patterns(), r))
        .collect();

    let mut watcher = DirWatcher::new(&WatcherConfig {
        poll_interval_ms: config.watcher.interval_millis,
        debounce_ms: config.watcher.debounce_ms,
        rename_timeout_ms: config.watcher.rename_timeout_ms,
        ..Default::default()
    })?;

    let watch_paths = config.watcher.effective_paths();
    for path in &watch_paths {
        watcher.watch(Path::new(path), config.watcher.recursive, config.watcher.max_depth)?;
    }

    config.print_startup_banner(&watch_paths);

    loop {
        if let Some(event) = watcher.next_event() {
            dispatch_event(&event, &compiled_rules);
        } else {
            error!("Watcher channel closed unexpectedly — exiting.");
            break Ok(());
        }
    }
}

/// Applies the first matching rule to a filesystem event.
fn dispatch_event(event: &FileEvent, compiled_rules: &[(Vec<Pattern>, &RuleConfig)]) {
    let path = match event {
        FileEvent::Created(p) | FileEvent::Modified(p) => p,
        FileEvent::Deleted(_) => return,
        FileEvent::Renamed { to, .. } => to,
    };

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Normalize to forward slashes so glob patterns work on Windows
    // (where Path::to_str() returns backslash-separated paths).
    let full = path
        .to_str()
        .unwrap_or("")
        .replace('\\', "/");

    // Use the first matching rule only. Without `break`, a second matching
    // rule would attempt to move an already-moved file and log a spurious
    // error.
    for (patterns, rule) in compiled_rules {
        let matched = patterns
            .iter()
            .any(|p| if p.raw.contains('/') { p.matches(&full) } else { p.matches(filename) });

        if matched {
            if let Err(e) = move_file(path, &rule.destination) {
                error!("Failed to move '{}': {e}", path.display());
            }
            break;
        }
    }
}

/// Polls `path` until its size has been stable across two consecutive checks
/// (`STABLE_POLL_INTERVAL` apart). Returns an error if the file disappears.
///
/// This ensures a file is fully written before it is moved. Downloads that
/// trigger a `Created`/`Modified` event early would otherwise be moved while
/// the writer still has the file open, producing a 0-byte destination.
///
/// Gives up and returns `Ok(())` after `STABLE_MAX_WAIT` to avoid blocking
/// the event loop indefinitely on a stalled download.
fn wait_until_stable(path: &Path) -> Result<(), AppError> {
    let started = Instant::now();
    let mut last_size: Option<u64> = None;

    loop {
        if started.elapsed() >= STABLE_MAX_WAIT {
            warn!(
                "Timed out waiting for '{}' to finish writing; moving it anyway.",
                path.display()
            );
            break;
        }

        match std::fs::metadata(path) {
            Ok(meta) => {
                let current_size = meta.len();
                if last_size == Some(current_size) {
                    // Size unchanged across two consecutive probes — file is
                    // stable.
                    break;
                }
                last_size = Some(current_size);
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::Other(format!("'{}' no longer exists", path.display())));
            },
            Err(e) => return Err(AppError::Io(e)),
        }

        std::thread::sleep(STABLE_POLL_INTERVAL);
    }

    Ok(())
}

/// Moves a file to the specified destination directory, creating it if needed.
///
/// Waits for the source file to stop growing before moving to avoid moving
/// partially-written downloads. Attempts an atomic rename first; falls back to
/// copy + delete when source and destination are on different filesystems.
fn move_file(from: &Path, to_dir: &str) -> Result<(), AppError> {
    // Wait for the file to be fully written before moving it. Without this,
    // a download that triggers a Created/Modified event early can be moved
    // while the writer still has it open, resulting in a 0-byte destination.
    wait_until_stable(from)?;

    let dest_dir = Path::new(to_dir);
    std::fs::create_dir_all(dest_dir)?;

    let filename = from
        .file_name()
        .ok_or_else(|| AppError::Other(format!("invalid filename: {}", from.display())))?;
    let dest = dest_dir.join(filename);

    // Try to rename first (fast, same filesystem)
    if std::fs::rename(from, &dest).is_err() {
        // Fall back to copy + delete (cross-filesystem)
        std::fs::copy(from, &dest)?;
        std::fs::remove_file(from)?;
    }

    info!("Moved {} → {}", from.display(), dest.display());
    Ok(())
}
