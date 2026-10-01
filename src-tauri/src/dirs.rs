//! Where files go: raw footage (downloads) and finished videos (exports) live in
//! separate folders, and everything is named by the time it was made.

use fillerncut_core::naming::{timestamp_stem, unique_path};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

fn base_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    app.path()
        .video_dir()
        .or_else(|_| app.path().download_dir())
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("FillernCut")
}

fn custom(setting: Option<String>) -> Option<PathBuf> {
    setting.filter(|d| !d.trim().is_empty()).map(PathBuf::from)
}

/// Downloaded, untouched footage. Default: ~/Movies/FillernCut/Footage
pub fn footage_dir<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> PathBuf {
    custom(state.settings().download_dir).unwrap_or_else(|| base_dir(app).join("Footage"))
}

/// Finished exports. Default: ~/Movies/FillernCut/Finished
pub fn finished_dir<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> PathBuf {
    custom(state.settings().export_dir).unwrap_or_else(|| base_dir(app).join("Finished"))
}

/// `2026-10-01_15-42-07` for the current local time.
pub fn now_stem() -> String {
    use chrono::{Datelike, Local, Timelike};
    let n = Local::now();
    timestamp_stem(n.year(), n.month(), n.day(), n.hour(), n.minute(), n.second())
}

/// A fresh, timestamp-named path with extension `ext` in `dir` (creating `dir`).
pub fn new_media_path(dir: &std::path::Path, ext: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    Ok(unique_path(dir, &now_stem(), ext))
}

/// A fresh, timestamp-named `.mp4` path in `dir`.
pub fn new_video_path(dir: &std::path::Path) -> std::io::Result<PathBuf> {
    new_media_path(dir, "mp4")
}
