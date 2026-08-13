use std::ffi::OsString;
use std::path::PathBuf;

use auto_launch::AutoLaunchBuilder;
use forgeconf::Config as _;

use crate::{AppError, art, logger};

/// The directory holding `rocas.toml` and, by default, the log file.
pub fn rocas_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rocas")
}

/// Where the config file is read from.
pub fn config_path() -> PathBuf {
    rocas_dir().join("rocas.toml")
}

fn downloads_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| {
        warn!("Could not determine downloads directory. Defaulting to current directory.");
        PathBuf::from(".")
    })
}

fn default_ignore() -> Vec<String> {
    ["*.part", "*.crdownload", "*.download", "*.partial", "*.tmp", "*.opdownload", ".~lock.*"]
        .map(String::from)
        .to_vec()
}

/// Builds a section whose fields all have defaults, for when its table is
/// absent from every source. The field attributes stay the only place a
/// default is written down.
fn all_defaults<T: forgeconf::FromNode>() -> T {
    forgeconf::from_str("", forgeconf::FileFormat::Toml)
        .expect("every field of the section has a default")
}

/// Builds the loader for the config file, the environment and `cli_args` (the
/// full argv, program name first).
pub fn loader(cli_args: Vec<OsString>) -> forgeconf::Loader<Config> {
    Config::loader().cli_args(cli_args)
}

/// Rocas watches directories and moves files into folders based on rules.
///
/// Also: -b, --boot toggles starting rocas on boot, then keeps running;
/// -V, --version prints the version.
#[derive(Debug, Clone, PartialEq, forgeconf::Config)]
#[config(file(path = config_path(), optional), env_prefix = "ROCAS_")]
pub struct Config {
    #[config(default)]
    pub watcher: WatcherConfig,

    pub rules: Vec<RuleConfig>,

    #[config(default)]
    pub misc: MiscConfig,
}

/// Configuration for the filesystem watcher.
#[derive(Debug, Clone, PartialEq, forgeconf::Config)]
pub struct WatcherConfig {
    /// Directory to watch; defaults to your Downloads directory
    ///
    /// Used when `watch_paths` is empty.
    pub watch_path: Option<PathBuf>,

    /// Multiple directories to watch simultaneously
    ///
    /// When non-empty this takes precedence over `watch_path`. All
    /// directories share the same `recursive`, `max_depth`, and timing
    /// settings.
    pub watch_paths: Vec<PathBuf>,

    /// Should rocas look into directories?
    #[config(default = false)]
    pub recursive: bool,

    /// How deep should rocas look into directories?
    ///
    /// Only available if recursive is enabled.
    pub max_depth: Option<usize>,

    /// Poll for changes instead of using OS notifications
    ///
    /// Turn this on for network drives, where the OS does not report
    /// changes. It costs a wake-up every `interval_millis`.
    #[config(default = false)]
    pub poll: bool,

    /// How often to poll for changes when `poll` is on (ms)
    ///
    /// Ignored unless `poll` is true.
    #[config(default = 1000)]
    pub interval_millis: u64,

    /// Collapse events within this window (ms)
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

    /// Move a file once its size and mtime stay unchanged this long (ms)
    ///
    /// Keeps rocas from moving a download that is still being written.
    #[config(default = 1000)]
    pub settle_ms: u64,

    /// Move a file anyway after waiting this long for it to settle (s)
    ///
    /// A warning is logged when this limit is hit.
    #[config(default = 300)]
    pub max_wait_secs: u64,

    /// Filename globs that are never moved (partial downloads, lock files)
    ///
    /// Defaults to *.part, *.crdownload, *.download, *.partial, *.tmp,
    /// *.opdownload and .~lock.*. Setting this replaces the whole list.
    #[config(default = default_ignore())]
    pub ignore: Vec<String>,

    /// Organize files already present in the watched directories at startup
    #[config(default = false)]
    pub scan_on_start: bool,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        all_defaults()
    }
}

impl WatcherConfig {
    /// Returns the effective list of directories to watch: `watch_paths` if
    /// it is non-empty, else `watch_path`, else the OS downloads directory.
    pub fn effective_paths(&self) -> Vec<PathBuf> {
        if !self.watch_paths.is_empty() {
            return self.watch_paths.clone();
        }
        vec![
            self.watch_path
                .clone()
                .unwrap_or_else(downloads_dir),
        ]
    }
}

/// Verbosity of the rocas log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, forgeconf::Config)]
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

/// What to do when a file with the same name already exists at the
/// destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, forgeconf::Config)]
#[config(rename_all = "lowercase")]
pub enum OnConflict {
    /// Keep both: the incoming file becomes "name (1).ext", "name (2).ext", ...
    Rename,
    /// Leave the incoming file where it is.
    Skip,
    /// Replace the existing file.
    Overwrite,
}

impl OnConflict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::Skip => "skip",
            Self::Overwrite => "overwrite",
        }
    }
}

/// Miscellaneous runtime configuration (logging, update checks).
#[derive(Debug, Clone, PartialEq, forgeconf::Config)]
pub struct MiscConfig {
    /// Should I look for updates on startup?
    ///
    /// Only available in builds with self-update (the GitHub release
    /// binaries); package-manager builds ignore it.
    #[config(default = true)]
    pub check_for_updates: bool,

    /// Should I auto update myself?
    ///
    /// Only available in builds with self-update.
    #[config(default = false)]
    pub auto_update: bool,

    /// Rocas log level
    #[config(default = LogLevel::Info)]
    pub log_level: LogLevel,

    /// Where should rocas store its log files; defaults to the OS config dir
    ///
    /// Omit to use the default path.
    /// Linux:   ~/.config/rocas/rocas.log
    /// macOS:   ~/Library/Application Support/rocas/rocas.log
    /// Windows: %APPDATA%\rocas\rocas.log
    pub log_file: Option<PathBuf>,

    /// Max limit of a log file size in megabytes
    ///
    /// Rotate the log file when it exceeds this size. 0 disables rotation.
    #[config(default = 10)]
    pub log_max_size_mb: u64,

    /// Number of log files to keep
    ///
    /// Number of rotated log files to keep alongside the active log
    /// (rocas.log.1, rocas.log.2, …).
    #[config(default = 3)]
    pub log_keep_files: u32,

    /// What to do when the destination already has a file with that name
    ///
    /// `rename` keeps both ("name (1).ext"), `skip` leaves the incoming file
    /// where it is, `overwrite` replaces the existing file.
    #[config(default = OnConflict::Rename)]
    pub on_conflict: OnConflict,

    /// Reload rocas.toml when it changes
    ///
    /// New rules and settings apply without restarting; an invalid file is
    /// reported and the previous configuration stays active.
    #[config(default = true)]
    pub reload: bool,
}

impl Default for MiscConfig {
    fn default() -> Self {
        all_defaults()
    }
}

impl MiscConfig {
    /// The log file: the configured one, else `rocas.log` in [`rocas_dir`].
    pub fn log_path(&self) -> PathBuf {
        self.log_file
            .clone()
            .unwrap_or_else(|| rocas_dir().join("rocas.log"))
    }
}

/// A single file-routing rule: files matching any `pattern` are moved to
/// `destination`.
#[derive(Debug, Clone, PartialEq, forgeconf::Config)]
pub struct RuleConfig {
    pub patterns: Vec<String>,
    pub destination: PathBuf,
}

impl Config {
    /// Builds and prints the startup ASCII art banner with configuration
    /// summary.
    pub fn print_startup_banner(&self, watch_paths: &[PathBuf]) {
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
        let w = &self.watcher;
        let m = &self.misc;

        msgs.push(dim("  watching"));
        msgs.push(info(&format!(
            "  {} director{} (v{})",
            watch_paths.len(),
            if watch_paths.len() == 1 { "y" } else { "ies" },
            env!("CARGO_PKG_VERSION")
        )));
        for path in watch_paths {
            msgs.push(info(&format!("    {}", path.display())));
        }

        msgs.push(String::new());
        msgs.push(dim("  watcher"));
        msgs.push(info(&format!(
            "  recursive={}  {}  debounce={}ms  rename_timeout={}ms{}",
            w.recursive,
            if w.poll { format!("poll={}ms", w.interval_millis) } else { "poll=off".to_string() },
            w.debounce_ms,
            w.rename_timeout_ms,
            match w.max_depth {
                Some(d) => format!("  max_depth={d}"),
                None => String::new(),
            }
        )));
        msgs.push(info(&format!(
            "  settle={}ms  max_wait={}s  scan_on_start={}",
            w.settle_ms, w.max_wait_secs, w.scan_on_start
        )));
        msgs.push(info(&format!("  ignore={}", w.ignore.join(", "))));

        msgs.push(String::new());
        msgs.push(dim("  rules"));
        if self.rules.is_empty() {
            msgs.push(info("  (none)"));
        } else {
            for rule in &self.rules {
                msgs.push(info(&format!(
                    "  {} → {}",
                    rule.patterns.join(", "),
                    rule.destination.display()
                )));
            }
        }

        msgs.push(String::new());
        msgs.push(dim("  misc"));
        msgs.push(info(&format!(
            "  log_level={}  on_conflict={}  reload={}  check_for_updates={}  auto_update={}",
            m.log_level
                .filter()
                .to_string()
                .to_lowercase(),
            m.on_conflict.as_str(),
            m.reload,
            m.check_for_updates,
            m.auto_update,
        )));
        msgs.push(info(&format!(
            "  log_file={}  max_size={}MB  keep={}",
            m.log_path().display(),
            m.log_max_size_mb,
            m.log_keep_files,
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

#[cfg(test)]
mod tests {
    use forgeconf::FileFormat;

    use super::*;

    fn parse(text: &str) -> Config {
        forgeconf::from_str(text, FileFormat::Toml).expect("test config is valid")
    }

    #[test]
    fn empty_file_yields_every_default() {
        let config = parse("");
        assert_eq!(
            config,
            Config {
                watcher: WatcherConfig {
                    watch_path: None,
                    watch_paths: vec![],
                    recursive: false,
                    max_depth: None,
                    poll: false,
                    interval_millis: 1000,
                    debounce_ms: 50,
                    rename_timeout_ms: 50,
                    settle_ms: 1000,
                    max_wait_secs: 300,
                    ignore: [
                        "*.part",
                        "*.crdownload",
                        "*.download",
                        "*.partial",
                        "*.tmp",
                        "*.opdownload",
                        ".~lock.*"
                    ]
                    .map(String::from)
                    .to_vec(),
                    scan_on_start: false,
                },
                rules: vec![],
                misc: MiscConfig {
                    check_for_updates: true,
                    auto_update: false,
                    log_level: LogLevel::Info,
                    log_file: None,
                    log_max_size_mb: 10,
                    log_keep_files: 3,
                    on_conflict: OnConflict::Rename,
                    reload: true,
                },
            }
        );
    }

    #[test]
    fn present_section_keeps_defaults_for_missing_keys() {
        let config = parse(
            "[watcher]\nrecursive = true\nsettle_ms = 250\n\n[misc]\non_conflict = \"skip\"\n",
        );
        assert!(config.watcher.recursive);
        assert_eq!(config.watcher.settle_ms, 250);
        assert_eq!(config.watcher.debounce_ms, 50);
        assert_eq!(config.misc.on_conflict, OnConflict::Skip);
        assert!(config.misc.reload);
    }

    #[test]
    fn on_conflict_accepts_every_variant() {
        for (text, expected) in [
            ("rename", OnConflict::Rename),
            ("skip", OnConflict::Skip),
            ("overwrite", OnConflict::Overwrite),
        ] {
            let config = parse(&format!("[misc]\non_conflict = \"{text}\"\n"));
            assert_eq!(config.misc.on_conflict, expected);
        }
    }

    #[test]
    fn ignore_replaces_the_default_list() {
        let config = parse("[watcher]\nignore = [\"*.bak\"]\n");
        assert_eq!(config.watcher.ignore, ["*.bak"]);
    }

    #[test]
    fn rules_load_with_path_destinations() {
        let config = parse("[[rules]]\npatterns = [\"*.pdf\"]\ndestination = \"/docs\"\n");
        assert_eq!(
            config.rules,
            [RuleConfig {
                patterns: vec!["*.pdf".into()],
                destination: PathBuf::from("/docs")
            }]
        );
    }

    #[test]
    fn watch_paths_win_over_watch_path() {
        let config = parse("[watcher]\nwatch_path = \"/a\"\nwatch_paths = [\"/b\", \"/c\"]\n");
        assert_eq!(config.watcher.effective_paths(), [PathBuf::from("/b"), PathBuf::from("/c")]);
    }

    #[test]
    fn watch_path_is_used_alone() {
        let config = parse("[watcher]\nwatch_path = \"/a\"\n");
        assert_eq!(config.watcher.effective_paths(), [PathBuf::from("/a")]);
    }

    #[test]
    fn nothing_set_watches_one_directory() {
        assert_eq!(
            parse("")
                .watcher
                .effective_paths()
                .len(),
            1
        );
    }

    #[test]
    fn explicit_log_file_is_used_verbatim() {
        let config = parse("[misc]\nlog_file = \"/tmp/x.log\"\n");
        assert_eq!(config.misc.log_path(), PathBuf::from("/tmp/x.log"));
    }

    #[test]
    fn default_log_file_lives_next_to_the_config() {
        assert_eq!(parse("").misc.log_path(), rocas_dir().join("rocas.log"));
    }

    #[test]
    fn log_levels_map_to_filters() {
        assert_eq!(LogLevel::Trace.filter(), log::LevelFilter::Trace);
        assert_eq!(LogLevel::Error.filter(), log::LevelFilter::Error);
    }
}
