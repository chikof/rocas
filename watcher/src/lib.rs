use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub enum WatchEvent {
    Created(PathBuf),
    Removed(PathBuf),
    Modified(PathBuf),
}

/// Metadata we track per file
#[derive(Clone, PartialEq)]
struct FileMeta {
    modified: SystemTime,
    size: u64,
}

pub struct WatcherConfig {
    /// Watch files inside subdirectories recursively
    pub recursive: bool,
    /// How often to poll the directory
    pub interval: Duration,
    /// Optional: max depth to recurse (None = unlimited)
    pub max_depth: Option<usize>,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            recursive: true,
            interval: Duration::from_millis(500),
            max_depth: None,
        }
    }
}

/// Snapshot of a directory: maps path -> metadata
type Snapshot = HashMap<PathBuf, FileMeta>;

fn snapshot(dir: &Path, config: &WatcherConfig, depth: usize) -> Snapshot {
    let mut map = HashMap::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    // Only recurse if recursive is enabled and we haven't hit max depth
                    let under_limit = config.max_depth.is_none_or(|max| depth < max);
                    if config.recursive && under_limit {
                        map.extend(snapshot(&path, config, depth + 1));
                    }
                } else if let Ok(modified) = meta.modified() {
                    map.insert(
                        path,
                        FileMeta {
                            modified,
                            size: meta.len(),
                        },
                    );
                }
            }
        }
    }

    map
}

fn diff(old: &Snapshot, new: &Snapshot, tx: &Sender<WatchEvent>) {
    // Files added or modified
    for (path, new_meta) in new {
        match old.get(path) {
            None => tx.send(WatchEvent::Created(path.clone())).ok(),
            Some(old_meta) if old_meta != new_meta => {
                tx.send(WatchEvent::Modified(path.clone())).ok()
            }
            _ => None,
        };
    }

    // Files removed
    for path in old.keys() {
        if !new.contains_key(path) {
            tx.send(WatchEvent::Removed(path.clone())).ok();
        }
    }
}

pub struct Watcher {
    pub rx: mpsc::Receiver<WatchEvent>,
}

impl Watcher {
    pub fn watch(dir: impl AsRef<Path>, config: WatcherConfig) -> Self {
        let dir = dir.as_ref().to_path_buf();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let mut prev = snapshot(&dir, &config, 0);
            loop {
                thread::sleep(config.interval);
                let curr = snapshot(&dir, &config, 0);
                diff(&prev, &curr, &tx);
                prev = curr;
            }
        });

        Watcher { rx }
    }
}
