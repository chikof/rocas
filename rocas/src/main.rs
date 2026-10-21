#![windows_subsystem = "windows"]

mod args;
mod art;
mod config;
mod engine;
mod logger;
mod mover;
mod pattern;
mod setup;
mod update;
mod watch;

#[macro_use]
extern crate log;

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
    if let setup::Flow::Exit = setup::dispatch(&args)? {
        return Ok(());
    }

    let loader = config::loader(args.rest);
    let config = loader.load_or_exit();

    if args.boot {
        config::startup_toggle()?;
    }

    logger::Logger::init(
        config.misc.log_level.filter(),
        Some(config.misc.log_path()),
        config.misc.log_max_size_mb,
        config.misc.log_keep_files,
    )?;

    engine::run(config, loader)
}
