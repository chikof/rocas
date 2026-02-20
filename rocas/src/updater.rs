use std::path::PathBuf;
use std::process::{self, Command};
use std::time::Duration;
use std::{fs, thread};

use crate::VERSION;

const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 10); // every 10 min
const RELEASES_API: &str = "https://api.github.com/repos/chikof/rocas/releases/latest";

pub struct Updater {
    pub current_version: &'static str,
}

impl Updater {
    pub fn new(current_version: &'static str) -> Self {
        Self { current_version }
    }

    /// Spawns a background thread that periodically checks for updates.
    /// If an update is found, it downloads it, spawns the new process, and
    /// exits.
    pub fn start_background_check(&self) {
        let current_version = self.current_version;

        thread::spawn(move || {
            loop {
                thread::sleep(UPDATE_CHECK_INTERVAL);

                match check_and_apply_update(current_version) {
                    Ok(true) => {
                        // New process has been spawned, exit this one
                        info!("Update applied, exiting current process");
                        process::exit(0);
                    },

                    Ok(false) => {
                        debug!(
                            "Already up to date, will check again in {} minutes",
                            UPDATE_CHECK_INTERVAL.as_secs() / 60
                        );
                    },

                    Err(e) => {
                        error!("Failed to check/apply update: {}", e);
                        info!("Will retry in {} minutes", UPDATE_CHECK_INTERVAL.as_secs() / 60)
                    },
                }
            }
        });
    }
}

pub fn current_platform_binary() -> &'static str {
    if cfg!(target_os = "windows") {
        "rocas.exe"
    } else if cfg!(target_os = "macos") {
        "rocas-macos"
    } else {
        "rocas-linux"
    }
}

fn check_and_apply_update(current_version: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let latest = fetch_latest_version()?;

    if latest.version == current_version {
        return Ok(false);
    }

    info!("Downloading from {}", latest.download_url);

    let current_exe = std::env::current_exe()?;
    let new_exe = current_exe.with_file_name("rocas_update.exe");

    // Download new binary
    download_file(&latest.download_url, &new_exe)?;

    // Spawn new process, passing --post-update so it knows to clean up
    Command::new(&new_exe)
        .arg("--post-update")
        .arg(
            current_exe
                .to_str()
                .unwrap(),
        )
        .spawn()?;

    Ok(true)
}

fn download_file(url: &str, dest: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("curl")
        .args([
            "-L", // follow redirects (important for GitHub releases)
            "-f", // fail on HTTP errors
            "-s", // silent
            "-o",
            dest.to_str()
                .unwrap(),
            url,
        ])
        .status()?;

    if !status.success() {
        return Err("Download failed".into());
    }

    // On Unix, make the binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dest, perms)?;
    }

    Ok(())
}

struct LatestRelease {
    version: String,
    download_url: String,
}

fn fetch_latest_version() -> Result<LatestRelease, Box<dyn std::error::Error>> {
    let output = Command::new("curl")
        .args([
            "-L",
            "-f",
            "-s",
            "-H",
            "User-Agent: rocas-updater", // GitHub API requires a User-Agent
            RELEASES_API,
        ])
        .output()?;

    let body = String::from_utf8(output.stdout)?;

    // Naive but dependency-free JSON field extraction
    let version = extract_json_string(&body, "tag_name").ok_or("missing tag_name")?;

    let binary_name = current_platform_binary();
    let download_url = find_asset_url(&body, binary_name).ok_or("no matching asset found")?;

    Ok(LatestRelease { version, download_url })
}

/// Extracts a string value from flat JSON: "key": "value"
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    let rest = rest
        .strip_prefix(':')?
        .trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;

    Some(rest[..end].to_string())
}

/// Finds browser_download_url for a specific asset name in the GitHub releases
/// JSON
fn find_asset_url(json: &str, asset_name: &str) -> Option<String> {
    // Find the asset block containing our binary name
    let name_needle = format!("\"name\": \"{}\"", asset_name);
    let asset_pos = json.find(&name_needle)?;
    // browser_download_url typically comes after the name field in the same asset
    // object
    let after = &json[asset_pos..];

    extract_json_string(after, "browser_download_url")
}

pub fn post_update_cleanup(old_exe_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let new_exe = std::env::current_exe()?;
    let target = std::path::Path::new(old_exe_path);

    std::thread::sleep(Duration::from_millis(500));

    #[cfg(unix)]
    {
        // On Unix we can just rename ourselves freely
        let _ = std::fs::remove_file(target);
        let _ = std::fs::rename(&new_exe, target);
    }

    #[cfg(windows)]
    {
        // On Windows we can't rename ourselves while running,
        // so we drop a cleanup .bat that runs after we're established
        let bat_path = target.with_file_name("rocas_cleanup.bat");
        let bat = format!(
            "@echo off\ntimeout /t 1 /nobreak > nul\ndel \"{}\"\ndel \"%~f0\"\n",
            old_exe_path
        );
        let _ = std::fs::write(&bat_path, bat);
        let _ = Command::new("cmd")
            .args([
                "/C",
                bat_path
                    .to_str()
                    .unwrap(),
            ])
            .spawn();
    }

    info!("Update completed, running v{}", VERSION);

    Ok(())
}
