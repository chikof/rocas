#![windows_subsystem = "windows"]

use std::time::Duration;

mod args;
mod art;
mod config;
mod engine;
mod logger;
mod mover;
mod pattern;
mod update;
mod watch;

#[macro_use]
extern crate log;

/// How often the config files are checked for changes.
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);

/// All errors that can occur in the rocas binary.
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("logger initialisation failed: {0}")]
    Logger(#[from] logger::LoggerInitError),

    #[error("watcher error: {0}")]
    Watcher(#[from] notify::Error),

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

    #[cfg(feature = "self-update")]
    #[error("{0}")]
    Other(String),
}

fn main() -> Result<(), AppError> {
    let args = args::parse(std::env::args_os().collect());
    if args.version {
        println!("rocas {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config = config::loader(args.rest.clone()).load_or_exit();

    if args.boot {
        config::startup_toggle()?;
    }

    logger::Logger::init(
        config.misc.log_level.filter(),
        Some(config.misc.log_path()),
        config.misc.log_max_size_mb,
        config.misc.log_keep_files,
    )?;

    if config.misc.check_for_updates {
        update::check(config.misc.auto_update)?;
    }

    let reload = config
        .misc
        .reload
        .then(|| config::loader(args.rest).watch(RELOAD_INTERVAL));
    engine::run(config, reload)
}
