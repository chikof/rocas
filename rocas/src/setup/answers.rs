//! What the wizard collects, and the defaults it starts from.

use std::path::PathBuf;

use crate::config::OnConflict;

/// The folders the presets point at.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub downloads: PathBuf,
    pub documents: PathBuf,
    pub pictures: PathBuf,
    pub videos: PathBuf,
    pub audio: PathBuf,
}

impl Dirs {
    /// The OS folders; one the OS does not define becomes a name under the
    /// home directory (or relative, without one).
    pub fn detect() -> Self {
        let home = dirs::home_dir();
        let under_home = |name: &str| {
            home.as_ref()
                .map_or_else(|| PathBuf::from(name), |h| h.join(name))
        };
        Self {
            downloads: dirs::download_dir().unwrap_or_else(|| under_home("Downloads")),
            documents: dirs::document_dir().unwrap_or_else(|| under_home("Documents")),
            pictures: dirs::picture_dir().unwrap_or_else(|| under_home("Pictures")),
            videos: dirs::video_dir().unwrap_or_else(|| under_home("Videos")),
            audio: dirs::audio_dir().unwrap_or_else(|| under_home("Music")),
        }
    }
}

/// One preset rule; only `enabled` ones reach the config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleChoice {
    pub name: String,
    pub patterns: Vec<String>,
    pub destination: PathBuf,
    pub enabled: bool,
}

/// Everything the wizard asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answers {
    pub watch_paths: Vec<PathBuf>,
    pub recursive: bool,
    pub rules: Vec<RuleChoice>,
    pub on_conflict: OnConflict,
    pub start_on_boot: bool,
    /// Only asked in builds with the `self-update` feature.
    pub check_for_updates: bool,
}

impl Answers {
    pub fn defaults(dirs: &Dirs) -> Self {
        let archives = dirs.downloads.join("Archives");
        let installers = dirs.downloads.join("Installers");
        Self {
            watch_paths: vec![dirs.downloads.clone()],
            recursive: false,
            rules: vec![
                preset(
                    "Documents",
                    &[
                        "pdf", "doc", "docx", "odt", "txt", "md", "rtf", "xls", "xlsx", "ods",
                        "ppt", "pptx", "odp", "csv",
                    ],
                    dirs.documents.clone(),
                ),
                preset(
                    "Images",
                    &["jpg", "jpeg", "png", "gif", "webp", "svg", "heic", "bmp", "tiff"],
                    dirs.pictures.clone(),
                ),
                preset("Videos", &["mp4", "mkv", "mov", "avi", "webm"], dirs.videos.clone()),
                preset("Audio", &["mp3", "flac", "wav", "ogg", "m4a", "opus"], dirs.audio.clone()),
                preset(
                    "Archives",
                    &["zip", "tar", "gz", "tgz", "xz", "bz2", "7z", "rar", "zst"],
                    archives,
                ),
                preset(
                    "Installers",
                    &["deb", "rpm", "AppImage", "dmg", "pkg", "exe", "msi", "flatpakref"],
                    installers,
                ),
            ],
            on_conflict: OnConflict::Rename,
            start_on_boot: false,
            check_for_updates: true,
        }
    }
}

fn preset(name: &str, extensions: &[&str], destination: PathBuf) -> RuleChoice {
    RuleChoice {
        name: name.to_owned(),
        patterns: globs(extensions),
        destination,
        enabled: true,
    }
}

/// `*.ext` globs. Matching is case-sensitive, so each extension also gets its
/// lower-case and upper-case spelling.
fn globs(extensions: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(extensions.len() * 2);
    for ext in extensions {
        for spelling in [(*ext).to_owned(), ext.to_lowercase(), ext.to_uppercase()] {
            let glob = format!("*.{spelling}");
            if !out.contains(&glob) {
                out.push(glob);
            }
        }
    }
    out
}

#[cfg(test)]
pub fn test_dirs() -> Dirs {
    let root = PathBuf::from("/home/u");
    Dirs {
        downloads: root.join("Downloads"),
        documents: root.join("Documents"),
        pictures: root.join("Pictures"),
        videos: root.join("Videos"),
        audio: root.join("Music"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_cover_both_cases() {
        assert_eq!(globs(&["pdf"]), ["*.pdf", "*.PDF"]);
        assert_eq!(globs(&["AppImage"]), ["*.AppImage", "*.appimage", "*.APPIMAGE"]);
    }

    #[test]
    fn defaults_watch_downloads_and_enable_every_preset() {
        let answers = Answers::defaults(&test_dirs());
        assert_eq!(answers.watch_paths, [PathBuf::from("/home/u/Downloads")]);
        assert_eq!(
            answers
                .rules
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["Documents", "Images", "Videos", "Audio", "Archives", "Installers"]
        );
        assert!(answers.rules.iter().all(|r| r.enabled));
        assert_eq!(answers.rules[4].destination, PathBuf::from("/home/u/Downloads/Archives"));
    }
}
