use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, select, tick};
use notify::event::{ModifyKind, RenameMode};
use notify::{
    Config,
    Event,
    EventKind,
    RecommendedWatcher,
    RecursiveMode,
    Result as NotifyResult,
    Watcher,
};
use rustc_hash::FxHashMap;

type WatchedRoots = Arc<std::sync::RwLock<Vec<(PathBuf, Option<usize>)>>>;

#[derive(Debug, Clone)]
pub enum FileEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

impl FileEvent {
    /// Returns the primary path associated with the event.
    pub fn path(&self) -> &Path {
        match self {
            FileEvent::Created(p) | FileEvent::Modified(p) | FileEvent::Deleted(p) => p,
            // For renames, the canonical "current" path is the destination.
            FileEvent::Renamed { to, .. } => to,
        }
    }
}

struct PendingRename {
    path: PathBuf,
    since: Instant,
}

impl PendingRename {
    fn new(path: PathBuf) -> Self {
        Self { path, since: Instant::now() }
    }

    /// A From event with no matching To within this window is treated as a
    /// delete.
    fn is_expired(&self, timeout: Duration) -> bool {
        self.since.elapsed() > timeout
    }
}

pub struct WatcherConfig {
    /// How many events the internal channel can buffer before backpressure
    /// kicks in.
    pub channel_capacity: usize,
    /// Events within this window for the same path are collapsed into one.
    pub debounce_ms: u64,
    /// How long to wait for a rename "To" before treating the "From" as a
    /// delete.
    pub rename_timeout_ms: u64,
    /// Polling interval passed to notify (relevant for the fallback poll
    /// backend).
    pub poll_interval_ms: u64,
    /// None = unlimited depth (fully recursive)
    /// Some(0) = only the watched root itself, no subdirectories
    /// Some(1) = root + one level of subdirectories, etc.
    pub max_depth: Option<usize>,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 1024,
            debounce_ms: 50,
            rename_timeout_ms: 50,
            poll_interval_ms: 50,
            max_depth: None,
        }
    }
}

pub struct DirWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<FileEvent>,
    // Maps each watched root → its configured max_depth
    watched_roots: WatchedRoots,
}

impl DirWatcher {
    pub fn new(config: WatcherConfig) -> NotifyResult<Self> {
        // Bounded channel between notify callback → translation thread.
        // Bounded gives backpressure instead of unbounded memory growth.
        let (raw_tx, raw_rx): (Sender<NotifyResult<Event>>, Receiver<NotifyResult<Event>>) =
            bounded(config.channel_capacity);

        // Bounded channel between translation thread → consumer.
        let (tx, rx): (Sender<FileEvent>, Receiver<FileEvent>) = bounded(config.channel_capacity);

        let rename_timeout = Duration::from_millis(config.rename_timeout_ms);
        let debounce_interval = Duration::from_millis(config.debounce_ms);

        let watched_roots: WatchedRoots = Arc::new(std::sync::RwLock::new(Vec::new()));
        let roots_for_thread = Arc::clone(&watched_roots);

        std::thread::Builder::new()
            .name("fs-event-translator".into())
            .spawn(move || {
                // Ticker drives the debounce flush window.
                let ticker = tick(debounce_interval);

                // Last event per path within the current debounce window.
                let mut pending: FxHashMap<PathBuf, FileEvent> =
                    FxHashMap::with_capacity_and_hasher(256, Default::default());

                let mut pending_rename: Option<PendingRename> = None;

                loop {
                    select! {
                        recv(raw_rx) -> msg => {
                            let event = match msg {
                                Ok(Ok(e)) => e,
                                Ok(Err(e)) => { log::error!("notify error: {e}"); continue; }
                                Err(_) => break, // channel closed, shut down
                            };

                            let roots = roots_for_thread.read().unwrap();

                            // Check if a stale pending rename should be emitted as a delete.
                            if let Some(ref r) = pending_rename
                                && r.is_expired(rename_timeout)
                            {
                                let r = pending_rename.take().unwrap();
                                pending.insert(r.path.clone(), FileEvent::Deleted(r.path));
                            }

                            match event.kind {
                                EventKind::Create(_) => {
                                    for path in event.paths {
                                        if path_allowed(&path, &roots) {
                                            pending.insert(path.clone(), FileEvent::Created(path));
                                        }
                                    }
                                }


                                EventKind::Modify(ModifyKind::Data(_))
                                | EventKind::Modify(ModifyKind::Metadata(_))
                                | EventKind::Modify(ModifyKind::Other) => {
                                    for path in event.paths {
                                        // Only downgrade to Modified if we haven't already
                                        // recorded a more significant event (Created).
                                        pending
                                            .entry(path.clone())
                                            .and_modify(|e| {
                                                if matches!(e, FileEvent::Created(_)) {
                                                    // A create + modify = still just a create.
                                                } else {
                                                    *e = FileEvent::Modified(path.clone());
                                                }
                                            })
                                            .or_insert_with(|| FileEvent::Modified(path));
                                    }
                                }

                                EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                                    if let Some(path) = event.paths.into_iter().next() {
                                        pending_rename = Some(PendingRename::new(path));
                                    }
                                }

                                EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                                    let to = event.paths.into_iter().next();

                                    // Expire check.
                                    let expired = pending_rename
                                        .as_ref()
                                        .map(|r| r.is_expired(rename_timeout))
                                        .unwrap_or(false);

                                    if expired
                                        && let Some(r) = pending_rename.take()
                                    {
                                        let key = r.path.clone();
                                        pending.insert(key, FileEvent::Deleted(r.path));
                                    }

                                    match (pending_rename.take(), to) {
                                        (Some(r), Some(to)) => {
                                            let key = to.clone();
                                            pending.insert(
                                                key,
                                                FileEvent::Renamed { from: r.path, to },
                                            );
                                        }
                                        // No matching From — treat To as a create.
                                        (None, Some(to)) => {
                                            pending.insert(
                                                to.clone(),
                                                FileEvent::Created(to),
                                            );
                                        }
                                        _ => {}
                                    }
                                }

                                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                                    let mut paths = event.paths.into_iter();
                                    if let (Some(from), Some(to)) = (paths.next(), paths.next()) {
                                        let key = to.clone();
                                        pending.insert(key, FileEvent::Renamed { from, to });
                                    }
                                }

                                EventKind::Remove(_) => {
                                    for path in event.paths {
                                        // A deleted file overrides any pending create/modify.
                                        pending.insert(path.clone(), FileEvent::Deleted(path));
                                    }
                                }

                                _ => {}
                            }
                        }

                        recv(ticker) -> _ => {
                            // Check for a rename that never got its To counterpart.
                            if let Some(ref r) = pending_rename
                                && r.is_expired(rename_timeout)
                            {
                                let r = pending_rename.take().unwrap();
                                pending.insert(r.path.clone(), FileEvent::Deleted(r.path));
                            }

                            for (_, event) in pending.drain() {
                                if tx.send(event).is_err() {
                                    // Consumer dropped; exit thread.
                                    return;
                                }
                            }
                        }
                    }
                }
            })
            .expect("failed to spawn fs-event-translator thread");

        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = raw_tx.send(res);
            },
            Config::default()
                .with_poll_interval(Duration::from_millis(config.poll_interval_ms))
                .with_compare_contents(false),
        )?;

        Ok(Self { watcher, receiver: rx, watched_roots })
    }

    /// Starts watching the specified path, optionally recursively.
    pub fn watch(
        &mut self,
        path: &Path,
        recursive: bool,
        max_depth: Option<usize>,
    ) -> NotifyResult<()> {
        let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        self.watcher.watch(path, mode)?;
        self.watched_roots
            .write()
            .unwrap()
            .push((path.to_path_buf(), max_depth));
        Ok(())
    }

    /// Stops watching the specified path.
    pub fn unwatch(&mut self, path: &Path) -> NotifyResult<()> {
        self.watcher.unwatch(path)
    }

    /// Blocks until the next debounced event is available.
    pub fn next_event(&self) -> Option<FileEvent> {
        self.receiver.recv().ok()
    }

    /// Non-blocking: drains all currently available debounced events.
    pub fn drain_events(&self) -> Vec<FileEvent> {
        self.receiver.try_iter().collect()
    }

    /// Returns a reference to the raw receiver so callers can integrate with
    /// their own `select!` or async bridge.
    pub fn receiver(&self) -> &Receiver<FileEvent> {
        &self.receiver
    }
}

/// Returns true if the path is within the allowed depth relative to root.
/// depth 0 = only files directly inside root
/// depth 1 = root + one subdir level, etc.
fn within_depth(root: &Path, path: &Path, max_depth: Option<usize>) -> bool {
    let max = match max_depth {
        None => return true, // unlimited
        Some(d) => d,
    };

    // Strip the root prefix to get the relative portion
    let relative = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return false, // path isn't under this root at all
    };

    // Count path components. A file directly in root has 1 component.
    // We subtract 1 so that depth=0 means "files directly in root".
    let components = relative.components().count();
    components > 0 && components - 1 <= max
}

/// Helper to check an event's paths against all watched roots
fn path_allowed(path: &Path, watched_roots: &[(PathBuf, Option<usize>)]) -> bool {
    watched_roots
        .iter()
        .any(|(root, max_depth)| within_depth(root, path, *max_depth))
}
