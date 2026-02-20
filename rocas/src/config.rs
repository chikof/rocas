#[allow(dead_code)]
use forgeconf::forgeconf;

use crate::pattern::Pattern;

pub fn downloads_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/Downloads", home)
}

#[forgeconf(config(path = "rocas.toml"))]
pub struct Config {
    #[field(name = "watcher")]
    pub watcher: WatcherConfig,

    #[field(name = "rules")]
    pub rules: Vec<RuleConfig>,
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
