//! The event loop and the state machine it drives.
//!
//! One thread owns everything. [`Tracker`] is a pure state machine: it is fed
//! events and the current time, and hands back [`Job`]s once a file has stopped
//! changing. [`run`] wires it to notify, the config watcher and the mover, and
//! sleeps in `recv`/`recv_timeout` until the next deadline, so an idle daemon
//! never wakes.

use std::cell::LazyCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};
use std::{fs, io, thread};

use forgeconf::ConfigError;
use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind};

use crate::AppError;
use crate::config::Config;
use crate::mover::{self, Moved};
use crate::pattern::Pattern;
use crate::watch::FsWatcher;

/// Everything the event loop waits on.
pub enum Msg {
    Fs(notify::Result<Event>),
    Reload(Result<Config, ConfigError>),
}

/// What the tracker needs to know about a path on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub is_file: bool,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

/// Filesystem lookups, injectable so the state machine runs without a disk.
pub trait Probe {
    fn stat(&self, path: &Path) -> io::Result<Stat>;
    fn canonicalize(&self, path: &Path) -> Option<PathBuf>;
}

struct FsProbe;

impl Probe for FsProbe {
    fn stat(&self, path: &Path) -> io::Result<Stat> {
        let meta = fs::metadata(path)?;
        Ok(Stat {
            is_file: meta.is_file(),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
        fs::canonicalize(path).ok()
    }
}

struct Root {
    path: PathBuf,
    canon: Option<PathBuf>,
    /// Deepest directory level whose files count; `Some(0)` = root only.
    max_depth: Option<usize>,
}

impl Root {
    fn admits(&self, path: &Path) -> bool {
        // Some backends report canonical paths even when watched through a
        // symlink, so both spellings of the root are tried.
        let rel = path
            .strip_prefix(&self.path)
            .ok()
            .or_else(|| {
                self.canon
                    .as_deref()
                    .and_then(|c| path.strip_prefix(c).ok())
            });
        rel.is_some_and(|rel| {
            let depth = rel.components().count();
            depth > 0
                && self
                    .max_depth
                    .is_none_or(|max| depth - 1 <= max)
        })
    }
}

struct Rule {
    patterns: Vec<Pattern>,
    dest: PathBuf,
    dest_canon: Option<PathBuf>,
}

impl Rule {
    fn is_dest(&self, dir: &Path) -> bool {
        dir == self.dest || self.dest_canon.as_deref() == Some(dir)
    }
}

/// A [`Config`] compiled for dispatch.
pub struct Plan {
    debounce: Duration,
    rename_timeout: Duration,
    settle: Duration,
    max_wait: Duration,
    roots: Vec<Root>,
    ignore: Vec<Pattern>,
    rules: Vec<Rule>,
}

impl Plan {
    pub fn from_config(config: &Config, probe: &impl Probe) -> Self {
        let w = &config.watcher;
        Self {
            debounce: Duration::from_millis(w.debounce_ms),
            rename_timeout: Duration::from_millis(w.rename_timeout_ms),
            settle: Duration::from_millis(w.settle_ms),
            max_wait: Duration::from_secs(w.max_wait_secs),
            roots: w
                .effective_paths()
                .into_iter()
                .map(|path| Root {
                    canon: probe
                        .canonicalize(&path)
                        .filter(|c| *c != path),
                    max_depth: if w.recursive { w.max_depth } else { Some(0) },
                    path,
                })
                .collect(),
            ignore: w
                .ignore
                .iter()
                .map(|p| Pattern::new(p))
                .collect(),
            rules: config
                .rules
                .iter()
                .map(|r| Rule {
                    patterns: r
                        .patterns
                        .iter()
                        .map(|p| Pattern::new(p))
                        .collect(),
                    dest_canon: probe
                        .canonicalize(&r.destination)
                        .filter(|c| *c != r.destination),
                    dest: r.destination.clone(),
                })
                .collect(),
        }
    }
}

/// A file that must be moved now.
#[derive(Debug, PartialEq, Eq)]
pub struct Job {
    pub from: PathBuf,
    pub dest: PathBuf,
}

/// A path seen recently whose burst of events has not been looked at yet.
struct Pending {
    deadline: Instant,
    last_event: Instant,
}

/// A file waiting to stop changing.
struct Candidate {
    stat: Stat,
    first_seen: Instant,
    last_change: Instant,
}

impl Candidate {
    fn due(&self, plan: &Plan) -> Instant {
        (self.last_change + plan.settle).min(self.first_seen + plan.max_wait)
    }
}

pub struct Tracker {
    plan: Plan,
    /// A rename's source, waiting for its destination event.
    pending_from: Option<(PathBuf, Instant)>,
    debounce: HashMap<PathBuf, Pending>,
    candidates: HashMap<PathBuf, Candidate>,
}

impl Tracker {
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            pending_from: None,
            debounce: HashMap::new(),
            candidates: HashMap::new(),
        }
    }

    /// Swaps in a new plan. Tracked files stay; the ones the new plan no
    /// longer wants are dropped when they come due.
    pub fn set_plan(&mut self, plan: Plan) {
        self.plan = plan;
    }

    /// The rule that `path` would be moved by, if any.
    fn resolve(&self, path: &Path) -> Option<usize> {
        let name = path.file_name()?.to_str()?;
        if !self
            .plan
            .roots
            .iter()
            .any(|r| r.admits(path))
        {
            return None;
        }
        if self
            .plan
            .ignore
            .iter()
            .any(|p| p.matches(name))
        {
            return None;
        }
        // Slash patterns see the whole path, normalized once and only if one
        // of them is tried.
        let full = LazyCell::new(|| {
            let s = path.to_string_lossy();
            if cfg!(windows) && s.contains('\\') { s.replace('\\', "/").into() } else { s }
        });
        let idx = self
            .plan
            .rules
            .iter()
            .position(|rule| {
                rule.patterns
                    .iter()
                    .any(|p| if p.wants_full_path() { p.matches(&full) } else { p.matches(name) })
            })?;
        // A file already in its rule's destination is home: skipping it is
        // what stops a destination inside a watched root from re-triggering.
        (!self.plan.rules[idx].is_dest(path.parent()?)).then_some(idx)
    }

    pub fn on_event(&mut self, event: Event, now: Instant, probe: &impl Probe) {
        let Event { kind, paths, .. } = event;
        match kind {
            EventKind::Create(_)
            | EventKind::Modify(
                ModifyKind::Data(_)
                | ModifyKind::Metadata(_)
                | ModifyKind::Any
                | ModifyKind::Other
                | ModifyKind::Name(RenameMode::Any | RenameMode::Other),
            ) => {
                for path in paths {
                    self.touch(path, now);
                }
            },
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                for path in paths {
                    self.vanish(&path);
                    self.pending_from = Some((path, now + self.plan.rename_timeout));
                }
            },
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                for path in paths {
                    let paired = self
                        .pending_from
                        .take()
                        .is_some_and(|(_, deadline)| deadline >= now);
                    if paired {
                        self.admit(path, now, probe);
                    } else {
                        self.touch(path, now);
                    }
                }
            },
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                let mut paths = paths.into_iter();
                if let (Some(from), Some(to)) = (paths.next(), paths.next()) {
                    self.vanish(&from);
                    self.admit(to, now, probe);
                }
            },
            EventKind::Remove(_) => {
                for path in paths {
                    self.vanish(&path);
                }
            },
            _ => {},
        }
    }

    /// Notes activity on `path`, collapsing bursts into one later look.
    fn touch(&mut self, path: PathBuf, now: Instant) {
        if let Some(pending) = self.debounce.get_mut(&path) {
            pending.last_event = now;
            return;
        }
        if !self.candidates.contains_key(&path) && self.resolve(&path).is_none() {
            return;
        }
        let deadline = now + self.plan.debounce;
        self.debounce
            .insert(path, Pending { deadline, last_event: now });
    }

    fn vanish(&mut self, path: &Path) {
        self.debounce.remove(path);
        self.candidates.remove(path);
    }

    /// Starts tracking `path` immediately, skipping the debounce: for
    /// renames, which are a single event, and for the startup scan.
    pub fn admit(&mut self, path: PathBuf, now: Instant, probe: &impl Probe) {
        self.debounce.remove(&path);
        self.observe(path, now, now, probe);
    }

    /// Looks at `path` on disk: starts tracking it, or updates what is known.
    fn observe(&mut self, path: PathBuf, seen: Instant, changed: Instant, probe: &impl Probe) {
        let stat = match probe.stat(&path) {
            Ok(stat) if stat.is_file => stat,
            _ => {
                self.candidates.remove(&path);
                return;
            },
        };
        if let Some(c) = self.candidates.get_mut(&path) {
            if c.stat != stat {
                c.stat = stat;
                c.last_change = changed;
            }
        } else if self.resolve(&path).is_some() {
            self.candidates.insert(
                path,
                Candidate {
                    stat,
                    first_seen: seen,
                    last_change: changed,
                },
            );
        }
    }

    /// Moves time forward: returns the files that have settled.
    pub fn advance(&mut self, now: Instant, probe: &impl Probe) -> Vec<Job> {
        let fired: Vec<_> = self
            .debounce
            .extract_if(|_, p| p.deadline <= now)
            .collect();
        for (path, pending) in fired {
            self.observe(path, pending.last_event, pending.last_event, probe);
        }

        let plan = &self.plan;
        let due: Vec<_> = self
            .candidates
            .extract_if(|_, c| c.due(plan) <= now)
            .collect();
        let mut jobs = Vec::new();
        for (path, mut c) in due {
            let Ok(stat) = probe.stat(&path) else { continue };
            if !stat.is_file {
                continue;
            }
            if stat != c.stat {
                if now < c.first_seen + self.plan.max_wait {
                    c.stat = stat;
                    c.last_change = now;
                    self.candidates.insert(path, c);
                    continue;
                }
                warn!(
                    "'{}' was still changing after {}s; moving it anyway.",
                    path.display(),
                    self.plan.max_wait.as_secs()
                );
            }
            let Some(idx) = self.resolve(&path) else { continue };
            let rule = &mut self.plan.rules[idx];
            if rule.dest_canon.is_none() {
                // The destination may only exist since the last move.
                rule.dest_canon = probe.canonicalize(&rule.dest);
            }
            jobs.push(Job { dest: rule.dest.clone(), from: path });
        }
        jobs
    }

    /// When [`advance`](Tracker::advance) next has something to do; `None`
    /// when idle.
    pub fn next_deadline(&self) -> Option<Instant> {
        let debounce = self
            .debounce
            .values()
            .map(|p| p.deadline)
            .min();
        let settle = self
            .candidates
            .values()
            .map(|c| c.due(&self.plan))
            .min();
        debounce.into_iter().chain(settle).min()
    }
}

struct Engine {
    config: Config,
    tracker: Tracker,
    fs: FsWatcher,
    watched: Vec<(PathBuf, bool)>,
    tx: Sender<Msg>,
    reload: bool,
}

fn new_fs(config: &Config, tx: &Sender<Msg>) -> notify::Result<FsWatcher> {
    let tx = tx.clone();
    let w = &config.watcher;
    FsWatcher::new(
        w.poll
            .then(|| Duration::from_millis(w.interval_millis)),
        move |res| {
            let _ = tx.send(Msg::Fs(res));
        },
    )
}

fn roots(config: &Config) -> Vec<(PathBuf, bool)> {
    config
        .watcher
        .effective_paths()
        .into_iter()
        .map(|p| (p, config.watcher.recursive))
        .collect()
}

impl Engine {
    fn watch_root(&mut self, path: PathBuf, recursive: bool) -> notify::Result<()> {
        self.fs.watch(&path, recursive)?;
        info!("Watching {} (recursive: {recursive})", path.display());
        if self.config.watcher.scan_on_start {
            self.scan(&path, recursive);
        }
        self.watched.push((path, recursive));
        Ok(())
    }

    /// Queues the files already present under `root`.
    fn scan(&mut self, root: &Path, recursive: bool) {
        let max = if recursive { self.config.watcher.max_depth } else { Some(0) };
        let now = Instant::now();
        let mut stack = vec![(root.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) => {
                    warn!("Cannot scan '{}': {e}", dir.display());
                    continue;
                },
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    if max.is_none_or(|max| depth < max) {
                        stack.push((entry.path(), depth + 1));
                    }
                } else {
                    self.tracker
                        .admit(entry.path(), now, &FsProbe);
                }
            }
        }
    }

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Fs(Ok(event)) => self
                .tracker
                .on_event(event, Instant::now(), &FsProbe),
            Msg::Fs(Err(e)) => error!("Watcher error: {e}"),
            Msg::Reload(_) if !self.reload => {},
            Msg::Reload(Ok(config)) => self.apply(config),
            Msg::Reload(Err(e)) => {
                error!("Config reload failed, keeping the previous configuration:\n{e}");
            },
        }
    }

    /// Switches to a freshly loaded configuration without a restart.
    fn apply(&mut self, new: Config) {
        if new == self.config {
            return;
        }
        let backend_changed = (new.watcher.poll, new.watcher.interval_millis)
            != (self.config.watcher.poll, self.config.watcher.interval_millis)
            && (new.watcher.poll || self.config.watcher.poll);

        self.tracker
            .set_plan(Plan::from_config(&new, &FsProbe));
        self.reload = new.misc.reload;
        let rules = new.rules.len();
        self.config = new;

        let want = roots(&self.config);
        let (mut added, mut removed) = (0, 0);
        if backend_changed {
            match new_fs(&self.config, &self.tx) {
                Ok(fs) => {
                    self.fs = fs;
                    self.watched.clear();
                },
                Err(e) => error!("Cannot switch the watch backend: {e}"),
            }
        }
        let gone: Vec<_> = self
            .watched
            .extract_if(.., |r| !want.contains(r))
            .collect();
        for (path, _) in gone {
            removed += 1;
            if let Err(e) = self.fs.unwatch(&path) {
                debug!("Unwatching '{}': {e}", path.display());
            }
        }
        for (path, recursive) in want {
            if self
                .watched
                .contains(&(path.clone(), recursive))
            {
                continue;
            }
            match self.watch_root(path.clone(), recursive) {
                Ok(()) => added += 1,
                Err(e) => error!("Cannot watch '{}': {e}", path.display()),
            }
        }
        info!(
            "Config reloaded: {rules} rule(s), {} director{} watched (+{added} -{removed})",
            self.watched.len(),
            if self.watched.len() == 1 { "y" } else { "ies" },
        );
    }

    fn execute(&self, job: &Job) {
        match mover::move_into(&job.from, &job.dest, self.config.misc.on_conflict) {
            Ok(Moved::Placed { to }) => info!("Moved {} → {}", job.from.display(), to.display()),
            Ok(Moved::Skipped) => info!(
                "Skipped '{}': '{}' already has a file with that name",
                job.from.display(),
                job.dest.display()
            ),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                debug!("'{}' vanished before it could be moved", job.from.display());
            },
            Err(e) => error!("Failed to move '{}': {e}", job.from.display()),
        }
    }
}

/// Runs the daemon until the process is stopped.
///
/// `reload` delivers freshly loaded configurations when the config file
/// changes; `None` disables hot reload.
///
/// # Errors
///
/// Fails if a watch directory cannot be watched at startup.
pub fn run(config: Config, reload: Option<forgeconf::Watcher<Config>>) -> Result<(), AppError> {
    let (tx, rx) = mpsc::channel();
    let fs = new_fs(&config, &tx)?;
    let tracker = Tracker::new(Plan::from_config(&config, &FsProbe));
    let reload_enabled = reload.is_some();
    let mut engine = Engine {
        config,
        tracker,
        fs,
        watched: Vec::new(),
        tx: tx.clone(),
        reload: reload_enabled,
    };

    for (path, recursive) in roots(&engine.config) {
        engine.watch_root(path, recursive)?;
    }
    let paths: Vec<PathBuf> = engine
        .watched
        .iter()
        .map(|(p, _)| p.clone())
        .collect();
    engine
        .config
        .print_startup_banner(&paths);

    if let Some(watcher) = reload {
        thread::Builder::new()
            .name("config-reload".into())
            .spawn(move || {
                while let Some(result) = watcher.recv() {
                    if tx.send(Msg::Reload(result)).is_err() {
                        break;
                    }
                }
            })?;
    } else {
        drop(tx);
    }

    loop {
        let msg = match engine.tracker.next_deadline() {
            None => rx
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
            Some(deadline) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
        };
        match msg {
            Ok(msg) => {
                engine.on_msg(msg);
                while let Ok(msg) = rx.try_recv() {
                    engine.on_msg(msg);
                }
            },
            Err(RecvTimeoutError::Timeout) => {},
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        for job in engine
            .tracker
            .advance(Instant::now(), &FsProbe)
        {
            engine.execute(&job);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use notify::event::{CreateKind, RemoveKind};

    use super::*;

    #[derive(Default)]
    struct Fake {
        files: RefCell<HashMap<PathBuf, Stat>>,
        stats: Cell<usize>,
    }

    impl Fake {
        fn put(&self, path: &str, len: u64, mtime: u64) {
            let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(mtime);
            self.files.borrow_mut().insert(
                path.into(),
                Stat {
                    is_file: true,
                    len,
                    modified: Some(modified),
                },
            );
        }

        fn dir(&self, path: &str) {
            self.files
                .borrow_mut()
                .insert(path.into(), Stat { is_file: false, len: 0, modified: None });
        }

        fn rm(&self, path: &str) {
            self.files
                .borrow_mut()
                .remove(Path::new(path));
        }
    }

    impl Probe for Fake {
        fn stat(&self, path: &Path) -> io::Result<Stat> {
            self.stats.set(self.stats.get() + 1);
            self.files
                .borrow()
                .get(path)
                .copied()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }

        fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
            path.to_str()
                .and_then(|p| p.strip_prefix("/link"))
                .map(|rest| PathBuf::from(format!("/real{rest}")))
        }
    }

    struct Rig {
        base: Instant,
        probe: Fake,
        tracker: Tracker,
    }

    fn plan(
        probe: &Fake,
        rules: &[(&[&str], &str)],
        recursive: bool,
        depth: Option<usize>,
    ) -> Plan {
        let mut config: Config = forgeconf::from_str("", forgeconf::FileFormat::Toml)
            .expect("an empty config has defaults");
        config.watcher.watch_paths = vec!["/in".into()];
        config.watcher.recursive = recursive;
        config.watcher.max_depth = depth;
        config.watcher.debounce_ms = 50;
        config.watcher.settle_ms = 1000;
        config.watcher.max_wait_secs = 10;
        config.rules = rules
            .iter()
            .map(|(patterns, dest)| crate::config::RuleConfig {
                patterns: patterns
                    .iter()
                    .map(|p| (*p).to_string())
                    .collect(),
                destination: (*dest).into(),
            })
            .collect();
        Plan::from_config(&config, probe)
    }

    fn rig(rules: &[(&[&str], &str)]) -> Rig {
        let probe = Fake::default();
        let tracker = Tracker::new(plan(&probe, rules, false, None));
        Rig { base: Instant::now(), probe, tracker }
    }

    fn ev(kind: EventKind, paths: &[&str]) -> Event {
        let mut e = Event::new(kind);
        for p in paths {
            e = e.add_path((*p).into());
        }
        e
    }

    fn created(path: &str) -> Event {
        ev(EventKind::Create(CreateKind::File), &[path])
    }

    fn modified(path: &str) -> Event {
        ev(EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)), &[path])
    }

    fn removed(path: &str) -> Event {
        ev(EventKind::Remove(RemoveKind::File), &[path])
    }

    fn rename(mode: RenameMode, paths: &[&str]) -> Event {
        ev(EventKind::Modify(ModifyKind::Name(mode)), paths)
    }

    fn job(from: &str, dest: &str) -> Job {
        Job { from: from.into(), dest: dest.into() }
    }

    impl Rig {
        fn at(&self, ms: u64) -> Instant {
            self.base + Duration::from_millis(ms)
        }

        fn event(&mut self, ms: u64, event: Event) {
            let now = self.at(ms);
            self.tracker
                .on_event(event, now, &self.probe);
        }

        fn advance(&mut self, ms: u64) -> Vec<Job> {
            let now = self.at(ms);
            self.tracker.advance(now, &self.probe)
        }

        fn deadline(&self) -> Option<u64> {
            self.tracker
                .next_deadline()
                .map(|d| u64::try_from((d - self.base).as_millis()).unwrap())
        }
    }

    const PDF: &[(&[&str], &str)] = &[(&["*.pdf"], "/docs")];

    #[test]
    fn a_new_file_moves_one_settle_after_its_last_event() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);

        r.event(0, created("/in/a.pdf"));
        assert_eq!(r.deadline(), Some(50));
        assert_eq!(r.advance(50), []);
        assert_eq!(r.deadline(), Some(1000));
        assert_eq!(r.advance(999), []);
        assert_eq!(r.advance(1000), [job("/in/a.pdf", "/docs")]);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn events_inside_the_debounce_window_collapse_into_one_stat() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);

        for ms in 0..5 {
            r.event(ms * 10, modified("/in/a.pdf"));
        }
        r.advance(50);

        assert_eq!(r.probe.stats.get(), 1);
        assert_eq!(r.deadline(), Some(1040));
    }

    #[test]
    fn a_growing_file_waits_until_it_stops_changing() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        r.probe.put("/in/a.pdf", 20, 2);
        r.event(500, modified("/in/a.pdf"));
        assert_eq!(r.advance(550), []);
        assert_eq!(r.advance(1000), []);
        assert_eq!(r.advance(1499), []);
        assert_eq!(r.advance(1500), [job("/in/a.pdf", "/docs")]);
    }

    #[test]
    fn a_change_without_any_event_is_caught_when_the_file_comes_due() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        r.probe.put("/in/a.pdf", 99, 1);
        assert_eq!(r.advance(1000), []);
        assert_eq!(r.deadline(), Some(2000));
        assert_eq!(r.advance(2000), [job("/in/a.pdf", "/docs")]);
    }

    #[test]
    fn a_file_that_never_stops_changing_moves_after_max_wait() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 1, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        let mut moved = Vec::new();
        for step in 1..=100u64 {
            let ms = step * 100;
            r.probe
                .put("/in/a.pdf", 1 + step, 1 + step);
            r.event(ms, modified("/in/a.pdf"));
            moved.extend(r.advance(ms));
            if !moved.is_empty() {
                assert_eq!(ms, 10_000);
                break;
            }
        }
        assert_eq!(moved, [job("/in/a.pdf", "/docs")]);
    }

    #[test]
    fn a_deleted_file_is_dropped() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        r.probe.rm("/in/a.pdf");
        r.event(100, removed("/in/a.pdf"));

        assert_eq!(r.deadline(), None);
        assert_eq!(r.advance(5000), []);
    }

    #[test]
    fn a_file_deleted_without_an_event_is_dropped_when_due() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        r.probe.rm("/in/a.pdf");
        assert_eq!(r.advance(1000), []);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn late_events_for_a_moved_file_are_harmless() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);
        assert_eq!(r.advance(1000).len(), 1);

        r.probe.rm("/in/a.pdf");
        r.event(1010, modified("/in/a.pdf"));
        r.event(1011, modified("/in/a.pdf"));

        assert_eq!(r.advance(1100), []);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn directories_are_never_moved() {
        let mut r = rig(&[(&["*"], "/docs")]);
        r.probe.dir("/in/sub");
        r.event(0, created("/in/sub"));

        assert_eq!(r.advance(50), []);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn renaming_away_drops_the_file() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 10, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        r.probe.rm("/in/a.pdf");
        r.event(100, rename(RenameMode::From, &["/in/a.pdf"]));

        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn a_partial_download_is_moved_after_the_rename_to_its_final_name() {
        let mut r = rig(&[(&["*"], "/docs")]);
        r.probe.put("/in/a.pdf.part", 5, 1);
        r.event(0, created("/in/a.pdf.part"));
        assert_eq!(r.deadline(), None);

        r.probe.rm("/in/a.pdf.part");
        r.probe.put("/in/a.pdf", 50, 2);
        r.event(2000, rename(RenameMode::From, &["/in/a.pdf.part"]));
        r.event(2001, rename(RenameMode::To, &["/in/a.pdf"]));

        assert_eq!(r.deadline(), Some(3001));
        assert_eq!(r.advance(3001), [job("/in/a.pdf", "/docs")]);
    }

    #[test]
    fn a_paired_rename_skips_the_debounce() {
        let mut r = rig(PDF);
        r.probe.put("/in/b.pdf", 5, 1);

        r.event(0, rename(RenameMode::From, &["/in/b.tmp"]));
        r.event(1, rename(RenameMode::To, &["/in/b.pdf"]));

        assert_eq!(r.deadline(), Some(1001));
    }

    #[test]
    fn a_rename_destination_without_its_source_is_debounced_like_a_create() {
        let mut r = rig(PDF);
        r.probe.put("/in/b.pdf", 5, 1);

        r.event(0, rename(RenameMode::To, &["/in/b.pdf"]));

        assert_eq!(r.deadline(), Some(50));
    }

    #[test]
    fn a_stale_rename_source_does_not_pair() {
        let mut r = rig(PDF);
        r.probe.put("/in/b.pdf", 5, 1);

        r.event(0, rename(RenameMode::From, &["/in/x"]));
        r.event(500, rename(RenameMode::To, &["/in/b.pdf"]));

        assert_eq!(r.deadline(), Some(550));
    }

    #[test]
    fn a_both_event_moves_tracking_to_the_new_name() {
        let mut r = rig(PDF);
        r.probe.put("/in/old.pdf", 5, 1);
        r.event(0, created("/in/old.pdf"));
        r.advance(50);

        r.probe.rm("/in/old.pdf");
        r.probe.put("/in/new.pdf", 5, 1);
        r.event(100, rename(RenameMode::Both, &["/in/old.pdf", "/in/new.pdf"]));

        assert_eq!(r.advance(1100), [job("/in/new.pdf", "/docs")]);
    }

    #[test]
    fn ignored_names_are_never_tracked() {
        let mut r = rig(&[(&["*"], "/docs")]);
        for name in [
            "a.part",
            "a.crdownload",
            "a.download",
            "a.partial",
            "a.tmp",
            "a.opdownload",
            ".~lock.a.odt#",
        ] {
            let path = format!("/in/{name}");
            r.probe.put(&path, 1, 1);
            r.event(0, created(&path));
        }

        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn files_nothing_matches_are_never_tracked() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.txt", 1, 1);
        r.event(0, created("/in/a.txt"));

        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn the_first_matching_rule_wins() {
        let mut r = rig(&[(&["*.pdf"], "/first"), (&["*"], "/second")]);
        r.probe.put("/in/a.pdf", 1, 1);
        r.probe.put("/in/a.txt", 1, 1);
        r.event(0, created("/in/a.pdf"));
        r.event(0, created("/in/a.txt"));
        r.advance(50);

        let mut jobs = r.advance(1000);
        jobs.sort_by(|a, b| a.from.cmp(&b.from));
        assert_eq!(jobs, [job("/in/a.pdf", "/first"), job("/in/a.txt", "/second")]);
    }

    #[test]
    fn slash_patterns_match_the_full_path() {
        let mut r = rig(&[(&["/in/rep*.pdf"], "/reports"), (&["*.pdf"], "/docs")]);
        r.probe.put("/in/report.pdf", 1, 1);
        r.probe.put("/in/other.pdf", 1, 1);
        r.event(0, created("/in/report.pdf"));
        r.event(0, created("/in/other.pdf"));
        r.advance(50);

        let mut jobs = r.advance(1000);
        jobs.sort_by(|a, b| a.from.cmp(&b.from));
        assert_eq!(jobs, [job("/in/other.pdf", "/docs"), job("/in/report.pdf", "/reports")]);
    }

    #[test]
    fn a_file_already_in_its_rules_destination_is_left_alone() {
        let probe = Fake::default();
        let mut tracker = Tracker::new(plan(&probe, &[(&["*.pdf"], "/in/docs")], true, None));
        probe.put("/in/docs/a.pdf", 1, 1);
        probe.put("/in/docs/deeper/a.pdf", 1, 1);
        let t0 = Instant::now();

        tracker.on_event(created("/in/docs/a.pdf"), t0, &probe);
        assert_eq!(tracker.next_deadline(), None);

        tracker.on_event(created("/in/docs/deeper/a.pdf"), t0, &probe);
        assert!(tracker.next_deadline().is_some());
    }

    #[test]
    fn the_destination_is_recognised_through_a_symlinked_spelling() {
        let probe = Fake::default();
        // The probe canonicalises "/link/docs" to "/real/docs", which is how
        // a backend may spell the same directory in its events.
        let mut plan = plan(&probe, &[(&["*.pdf"], "/link/docs")], true, None);
        plan.roots.push(Root {
            path: "/real".into(),
            canon: None,
            max_depth: None,
        });
        let mut tracker = Tracker::new(plan);
        probe.put("/real/docs/a.pdf", 1, 1);

        tracker.on_event(created("/real/docs/a.pdf"), Instant::now(), &probe);

        assert_eq!(tracker.next_deadline(), None);
    }

    #[test]
    fn a_destination_outside_the_root_does_not_matter() {
        let mut r = rig(PDF);
        r.probe.put("/docs/a.pdf", 1, 1);
        r.event(0, created("/docs/a.pdf"));

        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn non_recursive_roots_only_see_their_direct_children() {
        let mut r = rig(PDF);
        r.probe.put("/in/sub/a.pdf", 1, 1);
        r.event(0, created("/in/sub/a.pdf"));

        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn max_depth_limits_how_deep_files_count() {
        let probe = Fake::default();
        let mut tracker = Tracker::new(plan(&probe, PDF, true, Some(1)));
        let t0 = Instant::now();
        for path in ["/in/a.pdf", "/in/x/a.pdf", "/in/x/y/a.pdf"] {
            probe.put(path, 1, 1);
            tracker.on_event(created(path), t0, &probe);
        }
        tracker.advance(t0 + Duration::from_millis(50), &probe);

        let mut jobs = tracker.advance(t0 + Duration::from_millis(1050), &probe);
        jobs.sort_by(|a, b| a.from.cmp(&b.from));
        assert_eq!(jobs, [job("/in/a.pdf", "/docs"), job("/in/x/a.pdf", "/docs")]);
    }

    #[test]
    fn a_scanned_file_is_tracked_without_an_event() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 1, 1);

        let now = r.at(0);
        r.tracker
            .admit("/in/a.pdf".into(), now, &r.probe);

        assert_eq!(r.advance(1000), [job("/in/a.pdf", "/docs")]);
    }

    #[test]
    fn a_new_plan_drops_files_it_no_longer_wants() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 1, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        r.tracker
            .set_plan(plan(&r.probe, &[(&["*.txt"], "/docs")], false, None));

        assert_eq!(r.advance(1000), []);
    }

    #[test]
    fn a_new_plan_changes_the_destination_of_tracked_files() {
        let mut r = rig(PDF);
        r.probe.put("/in/a.pdf", 1, 1);
        r.event(0, created("/in/a.pdf"));
        r.advance(50);

        r.tracker
            .set_plan(plan(&r.probe, &[(&["*.pdf"], "/elsewhere")], false, None));

        assert_eq!(r.advance(1000), [job("/in/a.pdf", "/elsewhere")]);
    }

    #[test]
    fn an_idle_tracker_has_no_deadline() {
        let r = rig(PDF);
        assert_eq!(r.deadline(), None);
    }
}
