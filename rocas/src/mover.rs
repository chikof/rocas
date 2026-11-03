//! Moves a file into a directory without ever losing data.
//!
//! The destination is claimed atomically: a hard link fails with
//! `AlreadyExists` instead of replacing a file, so two movers (or the user)
//! racing for the same name cannot overwrite each other. Where hard links are
//! impossible (another filesystem, FAT, some network mounts) the file is
//! copied to a temporary file in the destination directory first and linked
//! into place from there.
//!
//! The source is only removed once the destination is complete: the copy is
//! flushed to disk and checked against the source having changed meanwhile,
//! it is linked into place, and the directory entry is flushed. A crash or an
//! error before that point leaves the source untouched and at worst a stray
//! `.rocas-*.tmp` file; after it, at worst both copies exist.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::OnConflict;

#[derive(Debug, PartialEq, Eq)]
pub enum Moved {
    /// The file now lives at `to`.
    Placed { to: PathBuf },
    /// A file with that name existed and `on_conflict = skip`; nothing changed.
    Skipped,
}

/// Moves `from` into `dest_dir`, creating the directory if needed.
///
/// # Errors
///
/// Fails if `from` has no file name, vanished, or the destination cannot be
/// written. On error `from` is left in place.
pub fn move_into(from: &Path, dest_dir: &Path, on_conflict: OnConflict) -> io::Result<Moved> {
    let name = from
        .file_name()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "path has no file name"))?;
    fs::create_dir_all(dest_dir)?;

    match on_conflict {
        OnConflict::Overwrite => {
            let to = dest_dir.join(name);
            replace(from, &to)?;
            Ok(Moved::Placed { to })
        },
        OnConflict::Skip => {
            let to = dest_dir.join(name);
            match place(from, &to) {
                Ok(()) => Ok(Moved::Placed { to }),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(Moved::Skipped),
                Err(e) => Err(e),
            }
        },
        OnConflict::Rename => {
            for n in 0u64.. {
                let to = dest_dir.join(numbered(name, n));
                match place(from, &to) {
                    Ok(()) => return Ok(Moved::Placed { to }),
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => {},
                    Err(e) => return Err(e),
                }
            }
            unreachable!("u64 candidate names are exhausted")
        },
    }
}

/// `name` for `n == 0`, otherwise `stem (n).ext`.
fn numbered(name: &OsStr, n: u64) -> OsString {
    if n == 0 {
        return name.to_os_string();
    }
    let path = Path::new(name);
    let mut out = path
        .file_stem()
        .unwrap_or(name)
        .to_os_string();
    out.push(format!(" ({n})"));
    if let Some(ext) = path.extension() {
        out.push(".");
        out.push(ext);
    }
    out
}

/// Moves `from` to `to`, failing with `AlreadyExists` if `to` exists.
fn place(from: &Path, to: &Path) -> io::Result<()> {
    match fs::hard_link(from, to) {
        Ok(()) => remove_source(from, to),
        Err(e) if matches!(e.kind(), ErrorKind::AlreadyExists | ErrorKind::NotFound) => Err(e),
        Err(_) => copy_into_place(from, to),
    }
}

/// Removes the source once the destination holds the file; if that fails the
/// destination is withdrawn so a failed move changes nothing.
fn remove_source(from: &Path, to: &Path) -> io::Result<()> {
    fs::remove_file(from).inspect_err(|_| {
        let _ = fs::remove_file(to);
    })
}

/// Copy fallback of [`place`]: temp file, fsync, no-clobber link, fsync of the
/// directory entry, and only then the source is dropped.
fn copy_into_place(from: &Path, to: &Path) -> io::Result<()> {
    let tmp = copy_to_temp(from, to)?;
    let linked = match fs::hard_link(&tmp, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(e),
        // No hard links here either. `rename` would replace silently, so
        // check first; this narrow window is the best a link-less filesystem
        // allows.
        Err(_) if fs::symlink_metadata(to).is_ok() => {
            Err(io::Error::new(ErrorKind::AlreadyExists, "destination exists"))
        },
        Err(_) => fs::rename(&tmp, to),
    };
    let _ = fs::remove_file(&tmp);
    linked?;
    sync_dir(to).inspect_err(|_| {
        let _ = fs::remove_file(to);
    })?;
    remove_source(from, to)
}

/// Flushes the directory holding `to`, so that once the source is gone a
/// crash cannot take the destination's name with it.
#[cfg_attr(
    not(unix),
    expect(clippy::unnecessary_wraps, reason = "fallible on Unix, where the directory is fsynced")
)]
fn sync_dir(to: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(
            to.parent()
                .unwrap_or_else(|| Path::new(".")),
        )?
        .sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = to;
        Ok(())
    }
}

/// Overwrite policy: replaces `to`, atomically where possible.
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(e),
        Err(_) => {
            let tmp = copy_to_temp(from, to)?;
            fs::rename(&tmp, to).inspect_err(|_| {
                let _ = fs::remove_file(&tmp);
            })?;
            sync_dir(to)?;
            fs::remove_file(from)
        },
    }
}

/// Copies `from` into a fresh temporary file next to `to`, flushed to disk.
fn copy_to_temp(from: &Path, to: &Path) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let dir = to
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let mut src = File::open(from)?;
    let meta = src.metadata()?;

    let (tmp, mut file) = loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".rocas-{}-{n}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {},
            Err(e) => return Err(e),
        }
    };

    let result = io::copy(&mut src, &mut file)
        .and_then(|_| ensure_unchanged(&src, &meta))
        .and_then(|()| file.set_permissions(meta.permissions()))
        .and_then(|()| match meta.modified() {
            Ok(time) => file.set_modified(time),
            Err(_) => Ok(()),
        })
        .and_then(|()| file.sync_all());
    match result {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        },
    }
}

/// Fails if `src` was written to since `before` was read: what was copied may
/// then be older than what is about to be deleted.
fn ensure_unchanged(src: &File, before: &Metadata) -> io::Result<()> {
    let now = src.metadata()?;
    if now.len() == before.len() && now.modified().ok() == before.modified().ok() {
        Ok(())
    } else {
        Err(io::Error::other("the file changed while it was being copied"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;

    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "rocas-mover-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, body: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, body).unwrap();
            path
        }

        fn read(&self, name: &str) -> String {
            fs::read_to_string(self.0.join(name)).unwrap()
        }

        fn names(&self) -> Vec<String> {
            let mut v: Vec<String> = fs::read_dir(&self.0)
                .unwrap()
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
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn plain_move_creates_the_destination() {
        let src = Dir::new();
        let dest = Dir::new();
        let from = src.file("a.txt", "one");
        let sub = dest.0.join("deep/er");

        let out = move_into(&from, &sub, OnConflict::Rename).unwrap();

        assert_eq!(out, Moved::Placed { to: sub.join("a.txt") });
        assert_eq!(fs::read_to_string(sub.join("a.txt")).unwrap(), "one");
        assert_eq!(src.names(), Vec::<String>::new());
    }

    #[test]
    fn rename_numbers_conflicts() {
        let src = Dir::new();
        let dest = Dir::new();
        dest.file("a.txt", "old");

        let first = move_into(&src.file("a.txt", "new1"), &dest.0, OnConflict::Rename).unwrap();
        let second = move_into(&src.file("a.txt", "new2"), &dest.0, OnConflict::Rename).unwrap();

        assert_eq!(first, Moved::Placed { to: dest.0.join("a (1).txt") });
        assert_eq!(second, Moved::Placed { to: dest.0.join("a (2).txt") });
        assert_eq!(dest.read("a.txt"), "old");
        assert_eq!(dest.read("a (1).txt"), "new1");
        assert_eq!(dest.read("a (2).txt"), "new2");
    }

    #[test]
    fn rename_handles_extensionless_dotfile_and_multi_dot_names() {
        let src = Dir::new();
        let dest = Dir::new();
        for name in ["README", ".bashrc", "a.tar.gz"] {
            dest.file(name, "old");
            move_into(&src.file(name, "new"), &dest.0, OnConflict::Rename).unwrap();
        }

        assert_eq!(
            dest.names(),
            [".bashrc", ".bashrc (1)", "README", "README (1)", "a.tar (1).gz", "a.tar.gz"]
        );
    }

    #[test]
    fn skip_leaves_both_files() {
        let src = Dir::new();
        let dest = Dir::new();
        dest.file("a.txt", "old");
        let from = src.file("a.txt", "new");

        let out = move_into(&from, &dest.0, OnConflict::Skip).unwrap();

        assert_eq!(out, Moved::Skipped);
        assert_eq!(dest.read("a.txt"), "old");
        assert_eq!(src.read("a.txt"), "new");
        assert_eq!(dest.names(), ["a.txt"]);
    }

    #[test]
    fn skip_moves_when_there_is_no_conflict() {
        let src = Dir::new();
        let dest = Dir::new();

        let out = move_into(&src.file("a.txt", "new"), &dest.0, OnConflict::Skip).unwrap();

        assert_eq!(out, Moved::Placed { to: dest.0.join("a.txt") });
        assert_eq!(src.names(), Vec::<String>::new());
    }

    #[test]
    fn overwrite_replaces_the_destination() {
        let src = Dir::new();
        let dest = Dir::new();
        dest.file("a.txt", "old");

        let out = move_into(&src.file("a.txt", "new"), &dest.0, OnConflict::Overwrite).unwrap();

        assert_eq!(out, Moved::Placed { to: dest.0.join("a.txt") });
        assert_eq!(dest.read("a.txt"), "new");
        assert_eq!(src.names(), Vec::<String>::new());
        assert_eq!(dest.names(), ["a.txt"]);
    }

    #[test]
    fn missing_source_is_an_error_and_creates_nothing() {
        let src = Dir::new();
        let dest = Dir::new();

        let err = move_into(&src.0.join("gone.txt"), &dest.0, OnConflict::Rename).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(dest.names(), Vec::<String>::new());
    }

    #[test]
    fn copy_fallback_never_clobbers_and_leaves_no_temp_files() {
        let src = Dir::new();
        let dest = Dir::new();
        let from = src.file("a.txt", "new");
        dest.file("a.txt", "old");

        let err = copy_into_place(&from, &dest.0.join("a.txt")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(dest.read("a.txt"), "old");
        assert_eq!(src.read("a.txt"), "new");
        assert_eq!(dest.names(), ["a.txt"]);

        copy_into_place(&from, &dest.0.join("b.txt")).unwrap();
        assert_eq!(dest.read("b.txt"), "new");
        assert_eq!(src.names(), Vec::<String>::new());
        assert_eq!(dest.names(), ["a.txt", "b.txt"]);
    }

    #[test]
    fn overwrite_copy_fallback_replaces_and_removes_the_source() {
        let src = Dir::new();
        let dest = Dir::new();
        let from = src.file("a.txt", "new");
        let to = dest.file("a.txt", "old");

        let tmp = copy_to_temp(&from, &to).unwrap();
        fs::rename(&tmp, &to).unwrap();

        assert_eq!(dest.read("a.txt"), "new");
        assert_eq!(dest.names(), ["a.txt"]);
    }

    #[test]
    fn concurrent_moves_of_same_named_files_lose_nothing() {
        const ROUNDS: usize = 200;
        let dest = Dir::new();
        let a = Dir::new();
        let b = Dir::new();

        let spawn = |src: &Dir, tag: &'static str| {
            let (src, dest) = (src.0.clone(), dest.0.clone());
            thread::spawn(move || {
                for i in 0..ROUNDS {
                    let dir = src.join(i.to_string());
                    fs::create_dir_all(&dir).unwrap();
                    let from = dir.join("x.txt");
                    fs::write(&from, format!("{tag}{i}")).unwrap();
                    move_into(&from, &dest, OnConflict::Rename).unwrap();
                }
            })
        };
        let (ta, tb) = (spawn(&a, "a"), spawn(&b, "b"));
        ta.join().unwrap();
        tb.join().unwrap();

        let mut bodies: Vec<String> = dest
            .names()
            .iter()
            .map(|n| dest.read(n))
            .collect();
        bodies.sort();
        let mut expected: Vec<String> = (0..ROUNDS)
            .flat_map(|i| [format!("a{i}"), format!("b{i}")])
            .collect();
        expected.sort();
        assert_eq!(bodies, expected);
    }

    #[test]
    fn a_copy_into_a_missing_directory_keeps_the_source() {
        let src = Dir::new();
        let dest = Dir::new();
        let from = src.file("a.txt", "new");

        let err = copy_into_place(&from, &dest.0.join("gone/a.txt")).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(src.read("a.txt"), "new");
        assert_eq!(dest.names(), Vec::<String>::new());
    }

    /// Makes `dir` read-only; `false` when that has no effect (running as
    /// root), in which case the test has nothing to show.
    #[cfg(unix)]
    fn lock(dir: &Dir) -> bool {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o555)).unwrap();
        let enforced = fs::write(dir.0.join(".probe"), "").is_err();
        if !enforced {
            unlock(dir);
            let _ = fs::remove_file(dir.0.join(".probe"));
        }
        enforced
    }

    #[cfg(unix)]
    fn unlock(dir: &Dir) {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_copy_into_a_read_only_directory_keeps_the_source() {
        let src = Dir::new();
        let dest = Dir::new();
        let from = src.file("a.txt", "new");
        if !lock(&dest) {
            return;
        }

        let err = copy_into_place(&from, &dest.0.join("a.txt")).unwrap_err();
        unlock(&dest);

        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        assert_eq!(src.read("a.txt"), "new");
        assert_eq!(dest.names(), Vec::<String>::new());
    }

    #[cfg(unix)]
    #[test]
    fn a_source_that_cannot_be_removed_stays_and_the_destination_is_withdrawn() {
        let src = Dir::new();
        let dest = Dir::new();
        let from = src.file("a.txt", "new");
        if !lock(&src) {
            return;
        }

        let err = move_into(&from, &dest.0, OnConflict::Rename).unwrap_err();
        unlock(&src);

        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        assert_eq!(src.read("a.txt"), "new");
        assert_eq!(dest.names(), Vec::<String>::new());
    }

    #[test]
    fn a_file_written_to_during_the_copy_is_detected() {
        let src = Dir::new();
        let from = src.file("a.txt", "new");
        let file = File::open(&from).unwrap();
        let before = file.metadata().unwrap();
        ensure_unchanged(&file, &before).unwrap();

        let mut writer = OpenOptions::new()
            .append(true)
            .open(&from)
            .unwrap();
        io::Write::write_all(&mut writer, b" more").unwrap();

        assert!(ensure_unchanged(&file, &before).is_err());
    }
}
