use std::path::Path;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
            Ok(_) => info!("Rocas will now start on boot."),
            Err(e) => error!("Failed to install autostart: {}", e),
        },

        Command::Unsetup => match auto()?.disable() {
            Ok(_) => info!("Autostart removed."),
            Err(e) => error!("Failed to remove autostart: {}", e),
        },

        Command::Run => run(&config)?,
    }

    Ok(())
}

fn run(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    if config.misc.check_for_updates {
        check_for_updates(config.misc.auto_update)?;
    }

    let compiled_rules: Vec<(Vec<Pattern>, &RuleConfig)> = config
        .rules
        .iter()
        .map(|r| (r.compiled_patterns(), r))
        .collect();

    let mut watcher = DirWatcher::new(WatcherConfig {
        poll_interval_ms: config.watcher.interval_millis,
        debounce_ms: config.watcher.debounce_ms,
        rename_timeout_ms: config.watcher.rename_timeout_ms,
        ..Default::default()
    })?;

    let watch_paths = config.watcher.effective_paths();
    for path in &watch_paths {
        watcher.watch(Path::new(path), config.watcher.recursive, config.watcher.max_depth)?;
    }

    // Build startup log messages to display alongside the ASCII art banner.
    // We format them the same way as the logger so the output is consistent.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ts = logger::format_timestamp(secs);

    let mut startup_msgs: Vec<String> = Vec::new();
    let tty = logger::stderr_is_tty();
    let dim =
        |s: &str| -> String { if tty { format!("\x1b[2m{s}\x1b[0m") } else { s.to_string() } };
    let info = |msg: &str| logger::format_line(&ts, log::Level::Info, "rocas", msg);

    startup_msgs.push(dim("  watching"));
    startup_msgs.push(info(&format!(
        "  {} director{} (v{})",
        watch_paths.len(),
        if watch_paths.len() == 1 { "y" } else { "ies" },
        cargo_crate_version!()
    )));
    for path in &watch_paths {
        startup_msgs.push(info(&format!("    {path}")));
    }

    startup_msgs.push(String::new());
    startup_msgs.push(dim("  watcher"));
    startup_msgs.push(info(&format!(
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

    startup_msgs.push(String::new());
    startup_msgs.push(dim("  rules"));
    if config.rules.is_empty() {
        startup_msgs.push(info("  (none)"));
    } else {
        for rule in &config.rules {
            startup_msgs.push(info(&format!(
                "  {} → {}",
                rule.patterns.join(", "),
                rule.destination
            )));
        }
    }

    startup_msgs.push(String::new());
    startup_msgs.push(dim("  misc"));
    startup_msgs.push(info(&format!(
        "  log_level={}  check_for_updates={}  auto_update={}",
        config.misc.log_level, config.misc.check_for_updates, config.misc.auto_update,
    )));
    startup_msgs.push(info(&format!(
        "  log_file={}  max_size={}MB  keep={}",
        config
            .misc
            .log_file
            .as_deref()
            .unwrap_or("(default)"),
        config.misc.log_max_size_mb,
        config.misc.log_keep_files,
    )));

    let msg_refs: Vec<&str> = startup_msgs
        .iter()
        .map(String::as_str)
        .collect();
    art::print_banner_with_messages(&msg_refs);

    loop {
        match watcher.next_event() {
            // returns Option<FileEvent>
            Some(event) => {
                let path = match &event {
                    FileEvent::Created(p) | FileEvent::Modified(p) => p,
                    FileEvent::Deleted(_) => continue,
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

                // Use the first matching rule only. Without `break`, a second
                // matching rule would attempt to move an already-moved file and
                // log a spurious error.
                for (patterns, rule) in &compiled_rules {
                    let matched = patterns.iter().any(|p| {
                        if p.raw.contains('/') { p.matches(&full) } else { p.matches(filename) }
                    });

                    if matched {
                        if let Err(e) = move_file(path, &rule.destination) {
                            error!("Failed to move '{}': {}", path.display(), e);
                        }
                        break;
                    }
                }
            },

            None => {
                error!("Watcher channel closed unexpectedly — exiting.");
                break Ok(());
            },
        }
    }
}

fn app_path() -> Result<String, Box<dyn std::error::Error>> {
    let path = std::env::current_exe()?;

    Ok(path.to_str().unwrap_or("").to_string())
}

fn auto() -> Result<AutoLaunch, Box<dyn std::error::Error>> {
    Ok(AutoLaunchBuilder::new()
        .set_app_name("Rocas")
        .set_app_path(&app_path()?)
        .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
        .set_windows_enable_mode(auto_launch::WindowsEnableMode::Dynamic)
        .set_linux_launch_mode(auto_launch::LinuxLaunchMode::Systemd)
        .build()?)
}

/// Moves a file to the specified destination directory, creating the directory
/// if it doesn't exist.
fn move_file(from: &Path, to_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dest_dir = Path::new(to_dir);
    std::fs::create_dir_all(dest_dir)?;

    let filename = from
        .file_name()
        .ok_or("invalid filename")?;
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

/// Checks for updates on GitHub and optionally auto-updates if a new version is
/// available.
fn check_for_updates(auto_update: bool) -> Result<(), Box<dyn std::error::Error>> {
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
