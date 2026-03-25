#![windows_subsystem = "windows"]

use std::path::Path;
use std::time::{Duration, Instant};

use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use cli::Command;
use config::{Config, RuleConfig};
use pattern::Pattern;
use self_update::cargo_crate_version;
use watcher::{DirWatcher, FileEvent, WatcherConfig};

mod art;
mod cli;
mod config;
mod logger;
mod pattern;

#[macro_use]
extern crate log;

/// How often to probe file size while waiting for a download to finish.
const STABLE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum time to wait for a file to stop growing before moving it anyway.
const STABLE_MAX_WAIT: Duration = Duration::from_secs(300);

/// All errors that can occur in the rocas binary.
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("failed to load config: {0}")]
    Config(#[from] forgeconf::ConfigError),

    #[error("logger initialisation failed: {0}")]
    Logger(#[from] logger::LoggerInitError),

    #[error("watcher error: {0}")]
    Watcher(#[from] watcher::Error),

    #[error("auto-launch error: {0}")]
    AutoLaunch(#[from] auto_launch::Error),

    #[error("update check failed: {0}")]
    Update(#[from] self_update::errors::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

fn main() -> Result<(), AppError> {
    let config = Config::loader().with_config().load()?;

    // Resolve the log file path: explicit config value, or the OS data dir.
    let log_path = config
        .misc
        .log_file
        .as_deref()
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("rocas").join("rocas.log")));

    logger::Logger::init(
        config.misc.log_level(),
        log_path,
        config.misc.log_max_size_mb,
        config.misc.log_keep_files,
    )?;

    match Command::from_args() {
        Command::Setup => match auto()?.enable() {
            Ok(()) => info!("Rocas will now start on boot."),
            Err(e) => error!("Failed to install autostart: {e}"),
        },

        Command::Unsetup => match auto()?.disable() {
            Ok(()) => info!("Autostart removed."),
            Err(e) => error!("Failed to remove autostart: {e}"),
        },

        Command::Run => run(&config)?,
    }

    Ok(())
}

fn run(config: &Config) -> Result<(), AppError> {
    if config.misc.check_for_updates {
        check_for_updates(config.misc.auto_update)?;
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

    print_startup_banner(config, &watch_paths);

    loop {
        if let Some(event) = watcher.next_event() {
            dispatch_event(&event, &compiled_rules);
        } else {
            error!("Watcher channel closed unexpectedly — exiting.");
            break Ok(());
        }
    }
}

/// Builds and prints the startup ASCII art banner with configuration summary.
fn print_startup_banner(config: &Config, watch_paths: &[&str]) {
    // We format messages the same way as the logger so the output is consistent.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ts = logger::format_timestamp(secs);

    let mut msgs: Vec<String> = Vec::new();
    let tty = logger::stderr_is_tty();
    let dim =
        |s: &str| -> String { if tty { format!("\x1b[2m{s}\x1b[0m") } else { s.to_string() } };
    let info = |msg: &str| logger::format_line(&ts, log::Level::Info, "rocas", msg);

    msgs.push(dim("  watching"));
    msgs.push(info(&format!(
        "  {} director{} (v{})",
        watch_paths.len(),
        if watch_paths.len() == 1 { "y" } else { "ies" },
        cargo_crate_version!()
    )));
    for path in watch_paths {
        msgs.push(info(&format!("    {path}")));
    }

    msgs.push(String::new());
    msgs.push(dim("  watcher"));
    msgs.push(info(&format!(
        "  recursive={}  interval={}ms  debounce={}ms  rename_timeout={}ms{}",
        config.watcher.recursive,
        config.watcher.interval_millis,
        config.watcher.debounce_ms,
        config.watcher.rename_timeout_ms,
        match config.watcher.max_depth {
            Some(d) => format!("  max_depth={d}"),
            None => String::new(),
        }
    )));

    msgs.push(String::new());
    msgs.push(dim("  rules"));
    if config.rules.is_empty() {
        msgs.push(info("  (none)"));
    } else {
        for rule in &config.rules {
            msgs.push(info(&format!("  {} → {}", rule.patterns.join(", "), rule.destination)));
        }
    }

    msgs.push(String::new());
    msgs.push(dim("  misc"));
    msgs.push(info(&format!(
        "  log_level={}  check_for_updates={}  auto_update={}",
        config.misc.log_level, config.misc.check_for_updates, config.misc.auto_update,
    )));
    msgs.push(info(&format!(
        "  log_file={}  max_size={}MB  keep={}",
        config
            .misc
            .log_file
            .as_deref()
            .unwrap_or("(default)"),
        config.misc.log_max_size_mb,
        config.misc.log_keep_files,
    )));

    let msg_refs: Vec<&str> = msgs
        .iter()
        .map(String::as_str)
        .collect();
    art::print_banner_with_messages(&msg_refs);
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
    // rule would attempt to move an already-moved file and log a spurious error.
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

fn app_path() -> Result<String, AppError> {
    let path = std::env::current_exe()?;
    // current_exe always returns a valid UTF-8 path on supported platforms;
    // fall back to empty string rather than failing hard.
    Ok(path.to_string_lossy().into_owned())
}

fn auto() -> Result<AutoLaunch, AppError> {
    Ok(AutoLaunchBuilder::new()
        .set_app_name("Rocas")
        .set_app_path(&app_path()?)
        .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
        .set_windows_enable_mode(auto_launch::WindowsEnableMode::Dynamic)
        .set_linux_launch_mode(auto_launch::LinuxLaunchMode::Systemd)
        .build()?)
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
                    // Size unchanged across two consecutive probes — file is stable.
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

/// Checks GitHub for a newer release and optionally performs an in-place
/// update.
fn check_for_updates(auto_update: bool) -> Result<(), AppError> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("chikof")
        .repo_name("rocas")
        .build()?
        .fetch()?;

    let latest = &releases[0];
    let current = cargo_crate_version!();

    if self_update::version::bump_is_greater(current, &latest.version)? {
        info!("New version available: {} → {}", current, latest.version);
        warn!(
            "To update manually update or set the 'misc.auto_update' option in the config to true."
        );

        if auto_update {
            self_update::backends::github::Update::configure()
                .repo_owner("chikof")
                .repo_name("rocas")
                .bin_name("rocas")
                .show_download_progress(true)
                .current_version(current)
                .build()?
                .update()?;

            info!("Updated to version {}", latest.version);
            info!("Please restart Rocas to apply the update.");
        }
    } else {
        trace!("No update available (current: {}, latest: {})", current, latest.version);
    }

    Ok(())
}
