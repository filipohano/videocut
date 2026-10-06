//! User settings, persisted as JSON in the app data directory.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    /// Check on launch; if a newer release exists, download and install it
    /// straight away and relaunch.
    Auto,
    /// Check on launch and show an "out of date" banner with an update button.
    Notify,
    /// Never check by itself; only when the user presses "Check now".
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CookieBrowser {
    Safari,
    Chrome,
    Firefox,
    Brave,
    Edge,
}

impl CookieBrowser {
    /// The name yt-dlp's `--cookies-from-browser` expects.
    pub fn ytdlp_name(self) -> &'static str {
        match self {
            CookieBrowser::Safari => "safari",
            CookieBrowser::Chrome => "chrome",
            CookieBrowser::Firefox => "firefox",
            CookieBrowser::Brave => "brave",
            CookieBrowser::Edge => "edge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub update_mode: UpdateMode,
    /// Keep the bundled downloader (yt-dlp) current — Instagram, X and TikTok
    /// break it regularly.
    pub auto_update_ytdlp: bool,
    /// Optional: let the downloader borrow the login from a browser.
    pub cookies_browser: Option<CookieBrowser>,
    /// Where finished exports go. `None` = ~/Movies/FillernCut/Finished.
    pub export_dir: Option<String>,
    /// Show a save dialog for every export instead of saving straight to the finished folder.
    pub ask_export_location: bool,
    /// Last used export quality (1..=100); 50 ≈ the source's own bitrate.
    pub export_quality: u8,
    /// JPEG quality for photo exports (1..=100).
    pub export_image_quality: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            update_mode: UpdateMode::Auto,
            auto_update_ytdlp: true,
            cookies_browser: None,
            export_dir: None,
            ask_export_location: false,
            export_quality: 60,
            export_image_quality: 90,
        }
    }
}

impl Settings {
    /// Read settings; a missing or corrupt file yields the defaults so a bad
    /// file can never stop the app from launching.
    pub fn load(path: &Path) -> Settings {
        fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    pub fn sanitized(mut self) -> Settings {
        self.export_quality = self.export_quality.clamp(1, 100);
        self.export_image_quality = self.export_image_quality.clamp(1, 100);
        self
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        write_json_atomic(path, self)
    }
}

/// Write JSON via a temp file + rename so a crash can't leave a half-written file.
pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_install_updates_automatically() {
        let s = Settings::default();
        assert_eq!(s.update_mode, UpdateMode::Auto);
        assert!(s.auto_update_ytdlp);
        assert_eq!(s.cookies_browser, None);
    }

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let s = Settings {
            update_mode: UpdateMode::Notify,
            cookies_browser: Some(CookieBrowser::Safari),
            export_dir: Some("/Users/me/Movies".into()),
            export_quality: 90,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
    }

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults_so_old_files_keep_working() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"updateMode":"manual"}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.update_mode, UpdateMode::Manual);
        assert_eq!(s.export_quality, 60);
        assert!(s.auto_update_ytdlp);
    }

    #[test]
    fn quality_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"exportQuality":0}"#).unwrap();
        assert_eq!(Settings::load(&path).export_quality, 1);
        fs::write(&path, r#"{"exportQuality":250}"#).unwrap();
        assert_eq!(Settings::load(&path).export_quality, 100);
    }

    #[test]
    fn json_shape_matches_what_the_frontend_sends() {
        let s: Settings = serde_json::from_str(
            r#"{"updateMode":"auto","autoUpdateYtdlp":false,"cookiesBrowser":"chrome",
                "exportDir":"/x","exportQuality":60}"#,
        )
        .unwrap();
        assert!(!s.auto_update_ytdlp);
        assert_eq!(s.cookies_browser, Some(CookieBrowser::Chrome));
        assert_eq!(CookieBrowser::Chrome.ytdlp_name(), "chrome");
    }
}
