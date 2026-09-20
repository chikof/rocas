//! Runs the real binary against temporary directories.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use std::{env, thread};

const SETTLE_MS: u64 = 200;

struct Sandbox {
    root: PathBuf,
    /// The watched directory; the inbox unless a test points it elsewhere.
    watch: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = env::temp_dir().join(format!(
            "rocas-e2e-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("home")).unwrap();
        fs::create_dir_all(root.join("inbox")).unwrap();
        Self { watch: root.join("inbox"), root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn inbox(&self) -> PathBuf {
        self.root.join("inbox")
    }

    fn docs(&self) -> PathBuf {
        self.root.join("docs")
    }

    fn log(&self) -> PathBuf {
        self.root.join("rocas.log")
    }

    fn config_file(&self) -> PathBuf {
        self.root.join("home/rocas/rocas.toml")
    }

    /// The text of rocas.toml: the inbox is watched, `rules` are `(pattern,
    /// destination)` pairs, `watcher` is extra `[watcher]` content.
    fn config_text(&self, watcher: &str, rules: &[(&str, &Path)]) -> String {
        // TOML literal strings keep backslashes in Windows paths intact.
        let mut toml = format!(
            "[watcher]\nwatch_paths = ['{}']\nsettle_ms = {SETTLE_MS}\n{watcher}\n",
            self.watch.display()
        );
        for (pattern, dest) in rules {
            let _ = write!(
                toml,
                "[[rules]]\npatterns = ['{pattern}']\ndestination = '{}'\n",
                dest.display()
            );
        }
        let _ = write!(
            toml,
            "[misc]\ncheck_for_updates = false\nlog_file = '{}'\n",
            self.log().display()
        );
        toml
    }

    /// Writes rocas.toml in place, as `echo > rocas.toml` would.
    fn configure(&self, watcher: &str, rules: &[(&str, &Path)]) {
        let path = self.config_file();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, self.config_text(watcher, rules)).unwrap();
    }

    /// Replaces rocas.toml the way editors save: write a temporary file, then
    /// rename it over the config.
    fn replace_config(&self, watcher: &str, rules: &[(&str, &Path)]) {
        let path = self.config_file();
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, self.config_text(watcher, rules)).unwrap();
        fs::rename(tmp, path).unwrap();
    }

    fn start(&self) -> Daemon {
        self.start_with(&[])
    }

    fn start_with(&self, args: &[&str]) -> Daemon {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rocas"));
        cmd.args(args);
        for (key, _) in env::vars_os() {
            if key
                .to_string_lossy()
                .starts_with("ROCAS_")
            {
                cmd.env_remove(key);
            }
        }
        let child = cmd
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home"))
            .env("XDG_DATA_HOME", self.root.join("home"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Daemon(child);
        self.wait_for_log("Watching");
        daemon
    }

    fn log_text(&self) -> String {
        fs::read_to_string(self.log()).unwrap_or_default()
    }

    fn wait_for_log(&self, needle: &str) {
        wait_until(&format!("log line {needle:?}"), || self.log_text().contains(needle));
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<String> = entries
        .map(|e| {
            e.unwrap()
                .file_name()
                .into_string()
                .unwrap()
        })
        .collect();
    v.sort();
    v
}

#[test]
fn a_dropped_file_is_moved() {
    let sb = Sandbox::new();
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let _d = sb.start();

    fs::write(sb.inbox().join("a.pdf"), "hello").unwrap();

    wait_until("a.pdf in docs", || sb.docs().join("a.pdf").exists());
    assert_eq!(fs::read_to_string(sb.docs().join("a.pdf")).unwrap(), "hello");
    assert_eq!(names(&sb.inbox()), Vec::<String>::new());
}

#[test]
fn files_nothing_matches_stay_put() {
    let sb = Sandbox::new();
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let _d = sb.start();

    fs::write(sb.inbox().join("a.txt"), "x").unwrap();
    fs::write(sb.inbox().join("b.pdf"), "y").unwrap();

    wait_until("b.pdf in docs", || sb.docs().join("b.pdf").exists());
    thread::sleep(Duration::from_millis(3 * SETTLE_MS));
    assert_eq!(names(&sb.inbox()), ["a.txt"]);
}

#[test]
fn a_growing_file_is_moved_only_after_it_stops_growing() {
    const CHUNKS: usize = 10;
    const CHUNK: usize = 1000;
    let sb = Sandbox::new();
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let _d = sb.start();

    let path = sb.inbox().join("big.pdf");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    for _ in 0..CHUNKS {
        file.write_all(&[b'x'; CHUNK]).unwrap();
        file.flush().unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(!sb.docs().join("big.pdf").exists(), "moved while still growing");
    }
    let finished = Instant::now();
    drop(file);

    wait_until("big.pdf in docs", || sb.docs().join("big.pdf").exists());
    assert!(finished.elapsed() >= Duration::from_millis(SETTLE_MS / 2));
    assert_eq!(
        fs::metadata(sb.docs().join("big.pdf"))
            .unwrap()
            .len(),
        (CHUNKS * CHUNK) as u64
    );
}

#[test]
fn a_partial_download_is_ignored_until_renamed() {
    let sb = Sandbox::new();
    sb.configure("", &[("*", &sb.docs())]);
    let _d = sb.start();

    let part = sb.inbox().join("x.pdf.part");
    fs::write(&part, "data").unwrap();
    thread::sleep(Duration::from_millis(4 * SETTLE_MS));
    assert!(part.exists());
    assert_eq!(names(&sb.docs()), Vec::<String>::new());

    fs::rename(&part, sb.inbox().join("x.pdf")).unwrap();

    wait_until("x.pdf in docs", || sb.docs().join("x.pdf").exists());
    assert_eq!(names(&sb.docs()), ["x.pdf"]);
    assert_eq!(names(&sb.inbox()), Vec::<String>::new());
}

#[test]
fn a_name_conflict_keeps_both_files() {
    let sb = Sandbox::new();
    sb.configure("", &[("*.pdf", &sb.docs())]);
    fs::create_dir_all(sb.docs()).unwrap();
    fs::write(sb.docs().join("x.pdf"), "old").unwrap();
    let _d = sb.start();

    fs::write(sb.inbox().join("x.pdf"), "new").unwrap();

    wait_until("x (1).pdf in docs", || sb.docs().join("x (1).pdf").exists());
    assert_eq!(fs::read_to_string(sb.docs().join("x.pdf")).unwrap(), "old");
    assert_eq!(fs::read_to_string(sb.docs().join("x (1).pdf")).unwrap(), "new");
    assert_eq!(names(&sb.docs()), ["x (1).pdf", "x.pdf"]);
}

#[test]
fn five_hundred_files_dropped_at_once_all_arrive() {
    const N: usize = 500;
    let sb = Sandbox::new();
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let _d = sb.start();

    for i in 0..N {
        fs::write(sb.inbox().join(format!("f{i}.pdf")), format!("body {i}")).unwrap();
    }

    wait_until("every file in docs", || {
        names(&sb.docs()).len() == N && names(&sb.inbox()).is_empty()
    });
    for i in 0..N {
        let body = fs::read_to_string(sb.docs().join(format!("f{i}.pdf"))).unwrap();
        assert_eq!(body, format!("body {i}"));
    }
}

#[test]
fn a_destination_inside_a_recursive_root_does_not_loop() {
    let sb = Sandbox::new();
    let dest = sb.inbox().join("docs");
    sb.configure("recursive = true", &[("*.pdf", &dest)]);
    let _d = sb.start();

    fs::write(sb.inbox().join("a.pdf"), "a").unwrap();
    fs::create_dir_all(sb.inbox().join("sub")).unwrap();
    wait_until("a.pdf in docs", || dest.join("a.pdf").exists());
    fs::write(sb.inbox().join("sub/b.pdf"), "b").unwrap();
    wait_until("b.pdf in docs", || dest.join("b.pdf").exists());
    thread::sleep(Duration::from_millis(5 * SETTLE_MS));

    assert_eq!(names(&dest), ["a.pdf", "b.pdf"]);
    assert_eq!(names(&sb.inbox()), ["docs", "sub"]);
    assert_eq!(fs::read_to_string(dest.join("a.pdf")).unwrap(), "a");
}

#[test]
fn files_already_present_are_organised_when_scan_on_start_is_set() {
    let sb = Sandbox::new();
    sb.configure("scan_on_start = true", &[("*.pdf", &sb.docs())]);
    fs::write(sb.inbox().join("old.pdf"), "old").unwrap();
    fs::write(sb.inbox().join("keep.txt"), "keep").unwrap();
    let _d = sb.start();

    wait_until("old.pdf in docs", || sb.docs().join("old.pdf").exists());
    assert_eq!(names(&sb.inbox()), ["keep.txt"]);
}

#[test]
fn editing_the_config_takes_effect_without_a_restart() {
    let sb = Sandbox::new();
    let texts = sb.root.join("texts");
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let _d = sb.start();

    sb.configure("", &[("*.pdf", &sb.docs()), ("*.txt", &texts)]);
    sb.wait_for_log("Config reloaded");
    fs::write(sb.inbox().join("n.txt"), "note").unwrap();

    wait_until("n.txt in texts", || texts.join("n.txt").exists());
    assert_eq!(fs::read_to_string(texts.join("n.txt")).unwrap(), "note");
}

#[test]
fn a_broken_config_edit_keeps_the_previous_rules() {
    let sb = Sandbox::new();
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let _d = sb.start();

    fs::write(sb.config_file(), "[[rules]\nnot toml at all").unwrap();
    sb.wait_for_log("Config reload failed");
    fs::write(sb.inbox().join("a.pdf"), "a").unwrap();

    wait_until("a.pdf in docs", || sb.docs().join("a.pdf").exists());
}

#[test]
fn deleting_and_recreating_the_config_reloads_it() {
    let sb = Sandbox::new();
    let texts = sb.root.join("texts");
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let _d = sb.start();

    fs::remove_file(sb.config_file()).unwrap();
    sb.wait_for_log("Config reload");
    sb.replace_config("", &[("*.pdf", &sb.docs()), ("*.txt", &texts)]);
    sb.wait_for_log("Config reloaded: 2 rule(s)");
    fs::write(sb.inbox().join("n.txt"), "note").unwrap();

    wait_until("n.txt in texts", || texts.join("n.txt").exists());
    assert_eq!(fs::read_to_string(texts.join("n.txt")).unwrap(), "note");
}

#[test]
fn the_config_file_is_never_organised_but_its_neighbours_are() {
    let mut sb = Sandbox::new();
    sb.watch = sb.home();
    let texts = sb.root.join("texts");
    sb.configure("recursive = true", &[("*.toml", &sb.docs())]);
    let _d = sb.start();

    sb.configure("recursive = true", &[("*.toml", &sb.docs()), ("*.txt", &texts)]);
    sb.wait_for_log("Config reloaded");
    fs::write(sb.home().join("other.toml"), "other").unwrap();

    wait_until("other.toml in docs", || sb.docs().join("other.toml").exists());
    thread::sleep(Duration::from_millis(3 * SETTLE_MS));
    assert_eq!(names(&sb.docs()), ["other.toml"]);
    assert!(
        fs::read_to_string(sb.config_file())
            .unwrap()
            .contains("*.txt")
    );
    assert_eq!(
        sb.log_text()
            .matches("Config reloaded")
            .count(),
        1,
        "{}",
        sb.log_text()
    );
}

#[test]
fn a_config_directory_created_after_startup_is_picked_up() {
    let sb = Sandbox::new();
    let inbox = sb.inbox();
    let log = sb.log();
    let mut d = sb.start_with(&[
        "--watcher.watch-path",
        inbox.to_str().unwrap(),
        "--misc.log-file",
        log.to_str().unwrap(),
        "--no-misc.check-for-updates",
    ]);
    let config_dir = sb
        .config_file()
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(!config_dir.exists(), "the daemon must not create the config directory");
    thread::sleep(Duration::from_millis(3 * SETTLE_MS));
    assert!(d.0.try_wait().unwrap().is_none(), "the daemon exited");

    sb.configure("", &[("*.pdf", &sb.docs())]);
    sb.wait_for_log("Config reloaded: 1 rule(s)");

    fs::write(inbox.join("late.pdf"), "l").unwrap();
    wait_until("late.pdf in docs", || sb.docs().join("late.pdf").exists());
}

#[test]
fn the_poll_backend_moves_a_dropped_file() {
    let sb = Sandbox::new();
    sb.configure("", &[("*.pdf", &sb.docs())]);
    let daemon = sb.start_with(&["--watcher.poll", "--watcher.interval-millis", "100"]);
    #[cfg(target_os = "linux")]
    assert!(!has_inotify(&daemon), "the poll backend must not use inotify");

    fs::write(sb.inbox().join("a.pdf"), "polled").unwrap();

    wait_until("a.pdf in docs", || sb.docs().join("a.pdf").exists());
    assert_eq!(fs::read_to_string(sb.docs().join("a.pdf")).unwrap(), "polled");
    assert_eq!(names(&sb.inbox()), Vec::<String>::new());
    drop(daemon);
}

/// Whether the daemon holds an inotify instance, which only the native backend
/// creates.
#[cfg(target_os = "linux")]
fn has_inotify(daemon: &Daemon) -> bool {
    fs::read_dir(format!("/proc/{}/fd", daemon.0.id()))
        .unwrap()
        .filter_map(|fd| fs::read_link(fd.unwrap().path()).ok())
        .any(|target| {
            target
                .to_string_lossy()
                .contains("inotify")
        })
}

/// Removes a directory tree on drop, for fixtures outside the sandbox.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[test]
fn a_move_across_filesystems_keeps_both_files_and_leaves_no_temp() {
    use std::os::unix::fs::MetadataExt as _;

    const SIZE: usize = 16 << 20;
    let shm = Path::new("/dev/shm");
    let mut sb = Sandbox::new();
    let dest = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "rocas-xdev-{}-{}",
        std::process::id(),
        sb.root
            .file_name()
            .unwrap()
            .to_string_lossy()
    ));
    fs::create_dir_all(&dest).unwrap();
    let _dest_cleanup = RemoveOnDrop(dest.clone());
    let same_device = fs::metadata(shm).map(|shm| shm.dev() == fs::metadata(&dest).unwrap().dev());
    if same_device.unwrap_or(true) {
        eprintln!("skipped: /dev/shm is missing or on the same device as {}", dest.display());
        return;
    }

    let inbox = shm.join(format!(
        "rocas-xdev-{}",
        sb.root
            .file_name()
            .unwrap()
            .to_string_lossy()
    ));
    fs::create_dir_all(&inbox).unwrap();
    let _inbox_cleanup = RemoveOnDrop(inbox.clone());
    sb.watch.clone_from(&inbox);
    sb.configure("", &[("*.txt", &dest)]);
    fs::write(dest.join("x.txt"), "old").unwrap();
    let _d = sb.start();

    let body: Vec<u8> = (0..SIZE)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let source = inbox.join("x.txt");
    fs::write(&source, &body).unwrap();

    let landed = dest.join("x (1).txt");
    wait_until("the source to be removed", || {
        if source.exists() {
            if let Ok(meta) = fs::metadata(&landed) {
                assert_eq!(meta.len(), SIZE as u64, "the destination appeared incomplete");
            }
            false
        } else {
            true
        }
    });

    assert_eq!(fs::read(&landed).unwrap(), body);
    assert_eq!(fs::read_to_string(dest.join("x.txt")).unwrap(), "old");
    assert_eq!(names(&dest), ["x (1).txt", "x.txt"]);
    assert_eq!(names(&inbox), Vec::<String>::new());
}
