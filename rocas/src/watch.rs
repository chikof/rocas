//! Thin adapter over `notify`: the platform's native backend, or the poll
//! backend for network drives where native events never arrive.

use std::path::Path;
use std::time::Duration;

use notify::{
    Config,
    EventHandler,
    PollWatcher,
    RecommendedWatcher,
    RecursiveMode,
    Result,
    Watcher,
};

pub enum FsWatcher {
    Native(RecommendedWatcher),
    Poll(PollWatcher),
}

impl FsWatcher {
    /// Builds a watcher delivering raw events to `handler`. `poll` is the poll
    /// interval when the poll backend is wanted, `None` for the native one.
    pub fn new(poll: Option<Duration>, handler: impl EventHandler) -> Result<Self> {
        match poll {
            Some(interval) => PollWatcher::new(
                handler,
                Config::default()
                    .with_poll_interval(interval)
                    .with_compare_contents(false),
            )
            .map(Self::Poll),
            None => RecommendedWatcher::new(handler, Config::default()).map(Self::Native),
        }
    }

    pub fn watch(&mut self, path: &Path, recursive: bool) -> Result<()> {
        let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        match self {
            Self::Native(w) => w.watch(path, mode),
            Self::Poll(w) => w.watch(path, mode),
        }
    }

    pub fn unwatch(&mut self, path: &Path) -> Result<()> {
        match self {
            Self::Native(w) => w.unwatch(path),
            Self::Poll(w) => w.unwatch(path),
        }
    }
}
