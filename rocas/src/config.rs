use std::path::PathBuf;

use auto_launch::AutoLaunchBuilder;
use forgeconf::Config as _;
use self_update::cargo_crate_version;

use crate::pattern::Pattern;
use crate::{AppError, art, logger};

pub fn downloads_path() -> String {
    let dir = dirs::download_dir();

    if let Some(dir) = dir {
        return dir.to_str().unwrap_or(".").to_string();
    }

    warn!("Could not determine downloads directory. Defaulting to current directory.");
    ".".to_string()
}

pub fn logs_path() -> String {
    let log_file = "rocas.log";

    rocas_dir()
        .join(log_file)
        .to_str()
        .unwrap_or(log_file)
        .to_string()
}

pub fn rocas_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or(std::path::PathBuf::from("."))
        .join("rocas")
}

pub fn config_path() -> String {
    let config_name = "rocas.toml";

    rocas_dir()
        .join(config_name)
        .to_str()
        .unwrap_or(config_name)
        .to_string()
}

/// Builds a section whose fields all have defaults, for when its table is
/// absent from every source.
fn all_defaults<T: forgeconf::FromNode>() -> T {
    forgeconf::from_str("", forgeconf::FileFormat::Toml)
        .expect("every field of the section has a default")
}

/// Rocas watches directories and moves files into folders based on rules.
///
/// Also: -b, --boot toggles starting rocas on boot, then keeps running;
/// -V, --version prints the version.
#[derive(forgeconf::Config)]
#[config(file(path = config_path(), optional), env_prefix = "ROCAS_")]
pub struct Config {
    #[config(default)]
    pub watcher: WatcherConfig,

    pub rules: Vec<RuleConfig>,

    #[config(default)]
    pub misc: MiscConfig,
}

/// Configuration for the filesystem watcher.
#[derive(forgeconf::Config)]
pub struct WatcherConfig {
    /// Specify the directory where rocas should look for new files
    ///
    /// Used when `watch_paths` is empty. Defaults to the OS downloads
    /// directory.
    #[config(default = downloads_path())]
    pub watch_path: String,

    /// Multiple directories to watch simultaneously
    ///
    /// When non-empty this takes precedence over `watch_path`. All
    /// directories share the same `recursive`, `max_depth`, and timing
    /// settings.
    pub watch_paths: Vec<String>,

    /// Should rocas look into directories?
    #[config(default = false)]
    pub recursive: bool,

    /// How often should rocas check for new files? (ms)
    #[config(default = 1000)]
    pub interval_millis: u64,

    /// How deep should rocas look into directories?
    ///
    /// Only available if recursive is enabled.
    pub max_depth: Option<usize>,

    /// Collapse events within this window (ms); increase for slow/network
    /// drives
    ///
    /// Events within this window for the same path are collapsed into one.
    /// Increase on slow network drives or when batch copy tools fire many
    /// rapid events.
    #[config(default = 50)]
    pub debounce_ms: u64,

    /// Wait this long for a rename pair before treating From as a delete (ms)
    ///
    /// How long to wait for a rename "To" counterpart before treating the
    /// "From" as a plain delete.
    #[config(default = 50)]
    pub rename_timeout_ms: u64,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        all_defaults()
    }
}

impl WatcherConfig {
    /// Returns the effective list of directories to watch. If `watch_paths` is
    /// non-empty it is used as-is; otherwise the single `watch_path` is
    /// returned as a one-element list.
    pub fn effective_paths(&self) -> Vec<&str> {
        if self.watch_paths.is_empty() {
            vec![self.watch_path.as_str()]
        } else {
            self.watch_paths
                .iter()
                .map(String::as_str)
                .collect()
        }
    }
}

/// Verbosity of the rocas log.
#[derive(Debug, Clone, Copy, forgeconf::Config)]
#[config(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// The matching [`log::LevelFilter`].
    pub const fn filter(self) -> log::LevelFilter {
        match self {
            Self::Trace => log::LevelFilter::Trace,
            Self::Debug => log::LevelFilter::Debug,
            Self::Info => log::LevelFilter::Info,
            Self::Warn => log::LevelFilter::Warn,
            Self::Error => log::LevelFilter::Error,
        }
    }
}

/// Miscellaneous runtime configuration (logging, update checks).
#[derive(forgeconf::Config)]
pub struct MiscConfig {
    /// Should I look for updates on startup?
    #[config(default = true)]
    pub check_for_updates: bool,

    /// Should I auto update myself?
    #[config(default = false)]
    pub auto_update: bool,

    /// Rocas log level
    #[config(default = LogLevel::Info)]
    pub log_level: LogLevel,

    /// Where should rocas store its log files
    ///
    /// Omit to use the OS default data directory.
    /// Linux:   ~/.local/share/rocas/rocas.log
    /// macOS:   ~/Library/Application Support/rocas/rocas.log
    /// Windows: %APPDATA%\rocas\rocas.log
    #[config(default = Some(logs_path()))]
    pub log_file: Option<String>,

    /// Max limit of a log file size in megabytes
    ///
    /// Rotate the log file when it exceeds this size.
    #[config(default = 10)]
    pub log_max_size_mb: u64,

    /// Number of log files to keep
    ///
    /// Number of rotated log files to keep alongside the active log
    /// (rocas.log.1, rocas.log.2, …).
    #[config(default = 3)]
    pub log_keep_files: u32,
}

impl Default for MiscConfig {
    fn default() -> Self {
        all_defaults()
    }
}

impl MiscConfig {
    /// The configured log level as a [`log::LevelFilter`].
    pub const fn log_level(&self) -> log::LevelFilter {
        self.log_level.filter()
    }
}

/// A single file-routing rule: files matching any `pattern` are moved to
/// `destination`.
#[derive(forgeconf::Config)]
pub struct RuleConfig {
    pub patterns: Vec<String>,
    pub destination: String,
}

impl RuleConfig {
    /// Compiles and returns all raw pattern strings as [`Pattern`] instances.
    ///
    /// Callers that match many files should call this once and retain the
    /// result rather than re-compiling on every match attempt.
    pub fn compiled_patterns(&self) -> Vec<Pattern> {
        self.patterns
            .iter()
            .map(|p| Pattern::new(p))
            .collect()
    }

    /// Returns `true` if any rule pattern matches `path`.
    ///
    /// Compiles patterns inline on each call; prefer [`compiled_patterns`] once
    /// and reusing the result in hot loops.
    ///
    /// [`compiled_patterns`]: RuleConfig::compiled_patterns
    // Retained for callers outside the main event loop (e.g. tests, future CLI).
    #[expect(dead_code, reason = "utility method kept for external callers and tests")]
    pub fn matches(&self, path: &str) -> bool {
        // Iterate directly over raw strings to avoid the intermediate Vec.
        self.patterns
            .iter()
            .any(|p| Pattern::new(p).matches(path))
    }
}

impl Config {
    /// Loads the configuration from the config file, the environment and
    /// `args` (the full argv), exiting on `--help` or on invalid input.
    pub fn load(args: Vec<std::ffi::OsString>) -> Self {
        Self::loader()
            .cli_args(args)
            .load_or_exit()
    }

    /// Builds and prints the startup ASCII art banner with configuration
    /// summary.
    pub fn print_startup_banner(&self, watch_paths: &[&str]) {
        // We format messages the same way as the logger so the output is
        // consistent.
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
            self.watcher.recursive,
            self.watcher.interval_millis,
            self.watcher.debounce_ms,
            self.watcher.rename_timeout_ms,
            match self.watcher.max_depth {
                Some(d) => format!("  max_depth={d}"),
                None => String::new(),
            }
        )));

        msgs.push(String::new());
        msgs.push(dim("  rules"));
        if self.rules.is_empty() {
            msgs.push(info("  (none)"));
        } else {
            for rule in &self.rules {
                msgs.push(info(&format!("  {} → {}", rule.patterns.join(", "), rule.destination)));
            }
        }

        msgs.push(String::new());
        msgs.push(dim("  misc"));
        msgs.push(info(&format!(
            "  log_level={}  check_for_updates={}  auto_update={}",
            self.misc
                .log_level
                .filter()
                .to_string()
                .to_lowercase(),
            self.misc.check_for_updates,
            self.misc.auto_update,
        )));
        msgs.push(info(&format!(
            "  log_file={}  max_size={}MB  keep={}",
            self.misc
                .log_file
                .as_deref()
                .unwrap_or("(default)"),
            self.misc.log_max_size_mb,
            self.misc.log_keep_files,
        )));

        let msg_refs: Vec<&str> = msgs
            .iter()
            .map(String::as_str)
            .collect();
        art::print_banner_with_messages(&msg_refs);
    }
}

pub fn startup_toggle() -> Result<(), AppError> {
    let conf = AutoLaunchBuilder::new()
        .set_app_name("Rocas")
        .set_app_path(&rocas_path()?)
        .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
        .set_windows_enable_mode(auto_launch::WindowsEnableMode::Dynamic)
        .set_linux_launch_mode(auto_launch::LinuxLaunchMode::Systemd)
        .build()?;

    if conf.is_enabled()? {
        conf.disable()?;
        info!("Fine, I didn't want to organize your shitty ass files anyway..");
    } else {
        conf.enable()?;
        info!("Gotchu boss, I'll be taking care of your files now.");
    }

    Ok(())
}

fn rocas_path() -> Result<String, AppError> {
    let path = std::env::current_exe()?;
    // current_exe always returns a valid UTF-8 path on supported platforms;
    // fall back to empty string rather than failing hard.
    Ok(path.to_string_lossy().into_owned())
}
