use std::path::Path;
use std::time::Duration;

use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use cli::Command;
use config::{Config, RuleConfig};
use pattern::Pattern;
use self_update::cargo_crate_version;
use watcher::{WatchEvent, Watcher};

mod cli;
mod config;
mod pattern;

#[macro_use]
extern crate log;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::loader().with_config().load()?;

    env_logger::Builder::new()
        .filter_level(config.misc.log_level()) // default level
        .parse_env("ROCAS_LOG") // can override with ROCAS_LOG=debug
        .format_timestamp_secs()
        .init();

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

    let watcher = Watcher::watch(
        &config.watcher.watch_path,
        watcher::WatcherConfig {
            recursive: config.watcher.recursive,
            interval: Duration::from_millis(config.watcher.interval_millis),
            max_depth: config.watcher.max_depth,
        },
    );

    info!("Watching {} (v{})", config.watcher.watch_path, cargo_crate_version!());

    for event in &watcher.rx {
        let path = match &event {
            WatchEvent::Created(p) | WatchEvent::Modified(p) => p,
            WatchEvent::Removed(_) => continue,
        };

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let full = path.to_str().unwrap_or("");

        for (patterns, rule) in &compiled_rules {
            let matched = patterns
                .iter()
                .any(|p| if p.raw.contains('/') { p.matches(full) } else { p.matches(filename) });

            if matched {
                info!("Matched '{}' -> moving to '{}'", path.display(), rule.destination);
                if let Err(e) = move_file(path, &rule.destination) {
                    error!("Failed to move '{}': {}'", path.display(), e);
                }
            }
        }
    }

    Ok(())
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

    // Try rename first (fast, same filesystem)
    if std::fs::rename(from, &dest).is_err() {
        // Fall back to copy + delete (cross-filesystem)
        std::fs::copy(from, &dest)?;
        std::fs::remove_file(from)?;
    }

    info!("Moved {} -> {}", from.display(), dest.display());
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
        info!("New version available: {} -> {}", current, latest.version);
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
