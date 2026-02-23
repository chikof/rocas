#[allow(dead_code)]
use forgeconf::forgeconf;

use crate::pattern::Pattern;

pub fn downloads_path() -> String {
    let dir = dirs::download_dir();

    if let Some(dir) = dir {
        return dir.to_str().unwrap_or(".").to_string();
    }

    warn!("Could not determine downloads directory. Defaulting to current directory.");
    ".".to_string()
}

pub fn config_path() -> String {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rocas");
    let config_name = "rocas.toml";

    dir.join(config_name)
        .to_str()
        .unwrap_or(config_name)
        .to_string()
}

#[forgeconf(config(path = config_path()))]
pub struct Config {
    #[field(name = "watcher")]
    pub watcher: WatcherConfig,

    #[field(name = "rules")]
    pub rules: Vec<RuleConfig>,

    #[field(name = "misc")]
    pub misc: MiscConfig,
}

#[forgeconf]
pub struct WatcherConfig {
    #[field(default = downloads_path())]
    pub watch_path: String,

    #[field(default = true)]
    pub recursive: bool,

    #[field(default = 1000)]
    pub interval_millis: u64,

    #[field(default = None)]
    pub max_depth: Option<usize>,
}

#[forgeconf]
pub struct MiscConfig {
    #[field(default = true)]
    pub check_for_updates: bool,

    #[field(default = false)]
    pub auto_update: bool,

    #[field(
        default = "info".to_string(),
        validate = forgeconf::validators::one_of(
            ["trace".to_string(), "debug".to_string(), "info".to_string(), "warn".to_string(), "error".to_string()]
        )
    )]
    pub log_level: String,
}

impl MiscConfig {
    pub fn log_level(&self) -> log::LevelFilter {
        match self.log_level.to_lowercase().as_str() {
            "trace" => log::LevelFilter::Trace,
            "debug" => log::LevelFilter::Debug,
            "info" => log::LevelFilter::Info,
            "warn" => log::LevelFilter::Warn,
            "error" => log::LevelFilter::Error,
            _ => log::LevelFilter::Info,
        }
    }
}

#[forgeconf]
pub struct RuleConfig {
    pub patterns: Vec<String>,
    pub destination: String,
}

impl RuleConfig {
    pub fn compiled_patterns(&self) -> Vec<Pattern> {
        self.patterns
            .iter()
            .map(|p| Pattern::new(p))
            .collect()
    }

    #[allow(dead_code)]
    pub fn matches(&self, path: &str) -> bool {
        self.compiled_patterns()
            .iter()
            .any(|p| p.matches(path))
    }
}
