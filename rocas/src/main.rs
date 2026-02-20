use std::path::Path;
use std::time::Duration;

use cli::Command;
use config::{Config, RuleConfig};
use pattern::Pattern;
use updater::Updater;
use watcher::{WatchEvent, Watcher};

mod autostart;
mod cli;
mod config;
mod pattern;
mod updater;

#[macro_use]
extern crate log;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info) // default level
        .parse_env("ROCAS_LOG") // can override with ROCAS_LOG=debug
        .format_timestamp_secs()
        .init();

    match Command::from_args() {
        Command::PostUpdate(old_exe) => {
            updater::post_update_cleanup(&old_exe)?;
        },

        Command::Setup => match autostart::install() {
            Ok(_) => log::info!("Rocas will now start on boot."),
            Err(e) => log::error!("Failed to install autostart: {}", e),
        },

        Command::Unsetup => match autostart::uninstall() {
            Ok(_) => info!("Autostart removed."),
            Err(e) => error!("Failed to remove autostart: {}", e),
        },

        Command::Run => run()?,
    }

    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Handle post-update cleanup before anything else
    if let Some(pos) = args
        .iter()
        .position(|a| a == "--post-update")
    {
        let old_exe = &args[pos + 1];
        updater::post_update_cleanup(old_exe)?;
    }

    let config = Config::loader()
        .with_config()
        .load()?;

    let compiled_rules: Vec<(Vec<Pattern>, &RuleConfig)> = config
        .rules
        .iter()
        .map(|r| (r.compiled_patterns(), r))
        .collect();

    // Start background update checker
    Updater::new(VERSION).start_background_check();

    let watcher = Watcher::watch(
        &config
            .watcher
            .watch_path,
        watcher::WatcherConfig {
            recursive: config
                .watcher
                .recursive,
            interval: Duration::from_millis(
                config
                    .watcher
                    .interval_millis,
            ),
            max_depth: config
                .watcher
                .max_depth,
        },
    );

    info!(
        "[rocas] Watching {} (v{})",
        config
            .watcher
            .watch_path,
        VERSION
    );

    for event in &watcher.rx {
        let path = match &event {
            WatchEvent::Created(p) | WatchEvent::Modified(p) => p,
            WatchEvent::Removed(_) => continue,
        };

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let full = path
            .to_str()
            .unwrap_or("");

        for (patterns, rule) in &compiled_rules {
            let matched = patterns
                .iter()
                .any(|p| {
                    if p.raw
                        .contains('/')
                    {
                        p.matches(full)
                    } else {
                        p.matches(filename)
                    }
                });

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
