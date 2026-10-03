//! Update checks against GitHub releases. Without the `self-update` feature
//! (the nix package), updates come from the package manager and this is a
//! no-op.

#[cfg(feature = "self-update")]
use std::path::Path;

#[cfg(feature = "self-update")]
use self_update::cargo_crate_version;

#[cfg(feature = "self-update")]
use crate::AppError;

/// Without self-update there is nothing to check.
#[cfg(not(feature = "self-update"))]
pub fn spawn(_auto_update: bool) {
    debug!("Built without self-update; updates come from the package manager.");
}

#[cfg(feature = "self-update")]
/// Replaces the current process image with a fresh instance of `exe`,
/// forwarding the original command-line arguments.
///
/// `exe` should be resolved with [`std::env::current_exe`] **before** calling
/// `.update()` so the path is captured while the original file is still open
/// and the OS inode / path mapping is unambiguous — particularly important on
/// Windows where `self_replace` renames the running file before writing the
/// new one.
///
/// On Unix this is a true `execv`: the kernel atomically replaces the process
/// image, the PID is unchanged, and no child process is created. It only
/// returns on error.
///
/// This runs on the update-check thread, not the main one. `execv` replaces
/// the whole process whichever thread calls it: the event loop, the watcher
/// and every other thread are gone the instant it succeeds, and the new image
/// starts from scratch (a fresh watcher, a fresh config load). A move cut off
/// mid-copy loses nothing, as the mover removes the source only after the
/// destination is complete; the file is picked up again when it next changes
/// or, with `scan_on_start`, at startup.
///
/// On Windows `execv` does not exist. A new process is spawned with
/// `CreateProcess` and the current one exits with code 0 — the closest safe
/// equivalent available without platform-specific unsafe code.
///
/// # Errors
///
/// Returns [`AppError::Restart`] if (Unix) `execv` fails. The Windows spawn
/// path calls `std::process::exit` on success and only returns on failure.
fn restart(exe: &Path) -> Result<(), AppError> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // exec replaces the current process image; it only returns on error.
        let err = std::process::Command::new(exe)
            .args(&args)
            .exec();
        Err(AppError::Restart(format!("exec failed: {err}")))
    }

    #[cfg(not(unix))]
    {
        use std::os::windows::process::CommandExt as _;

        // DETACHED_PROCESS (0x00000008): the new process gets no console of
        // its own and is fully independent of this one. Combined with
        // `into_raw_handle()` this ensures the child's lifetime is not tied
        // to the parent's process object, so calling `exit(0)` immediately
        // afterwards cannot race with child initialisation.
        //
        // We intentionally leak the raw handle — we are about to exit anyway
        // and the OS will reclaim it. The explicit leak makes the intent
        // clear: we do not want to wait on or terminate the child.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let child = std::process::Command::new(exe)
            .args(&args)
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .map_err(|e| AppError::Restart(format!("spawn failed: {e}")))?;
        std::mem::forget(child);
        std::process::exit(0);
    }
}

#[cfg(feature = "self-update")]
/// Runs [`check`] on a background thread so a slow or absent network never
/// delays watching. Failures are logged, not fatal.
pub fn spawn(auto_update: bool) {
    let started = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            if let Err(e) = check(auto_update) {
                warn!("Update check failed: {e}");
            }
        });
    if let Err(e) = started {
        warn!("Cannot start the update check: {e}");
    }
}

#[cfg(feature = "self-update")]
/// Checks GitHub for a newer release and optionally performs an in-place
/// update; an update ends in [`restart`], which does not return on success.
fn check(auto_update: bool) -> Result<(), AppError> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("chikof")
        .repo_name("rocas")
        .build()?
        .fetch()?;

    let latest = releases
        .first()
        .ok_or_else(|| AppError::Other("GitHub returned an empty release list".to_string()))?;
    let current = cargo_crate_version!();

    if self_update::version::bump_is_greater(current, &latest.version)? {
        info!("New version available: {} → {}", current, latest.version);
        warn!(
            "To update manually update or set the 'misc.auto_update' option in the config to true."
        );

        if auto_update {
            // Capture the exe path before the update so the path is resolved
            // while the current file is still open (matters on Windows where
            // self_replace renames the running exe before writing the new one).
            let exe = std::env::current_exe()?;

            self_update::backends::github::Update::configure()
                .repo_owner("chikof")
                .repo_name("rocas")
                .bin_name("rocas")
                .show_download_progress(true)
                .current_version(current)
                .build()?
                .update()?;

            info!("Updated to version {}. Restarting…", latest.version);
            restart(&exe)?;
        }
    } else {
        trace!("No update available (current: {}, latest: {})", current, latest.version);
    }

    Ok(())
}
