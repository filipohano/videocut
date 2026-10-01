//! Settings commands and yt-dlp maintenance.

use fillerncut_core::ytdlp::checksum_for;
use fillerncut_core::Settings;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::State;

use crate::state::AppState;
use crate::{bins, jobs};

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings, String> {
    let settings = settings.sanitized();
    settings
        .save(&state.settings_path)
        .map_err(|e| format!("Couldn't save settings: {e}"))?;
    *state.settings.lock().unwrap() = settings.clone();
    Ok(settings)
}

const YTDLP_RELEASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The downloader (yt-dlp) isn't bundled: it's a self-contained Python app whose
/// signature breaks when re-signed into our bundle. Instead it's downloaded from
/// the official release on first use (checksum-verified) and kept up to date.
pub async fn ensure_ytdlp(data_dir: &Path) -> Result<PathBuf, String> {
    let _guard = INSTALL_LOCK.lock().await;
    let existing = bins::ytdlp(data_dir);
    if existing.is_absolute() && existing.is_file() {
        return Ok(existing);
    }
    install_ytdlp(data_dir).await
}

async fn install_ytdlp(data_dir: &Path) -> Result<PathBuf, String> {
    let offline = "Couldn't download the downloader (yt-dlp). Check your internet connection and try again";
    let client = reqwest::Client::builder()
        .user_agent("FillernCut")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let get = |name: &str| client.get(format!("{YTDLP_RELEASE}/{name}")).send();

    let sums = get("SHA2-256SUMS")
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| offline.to_string())?
        .text()
        .await
        .map_err(|_| offline.to_string())?;
    let expected =
        checksum_for(&sums, "yt-dlp_macos").ok_or("yt-dlp's checksum list is missing yt-dlp_macos")?;
    let bytes = get("yt-dlp_macos")
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| offline.to_string())?
        .bytes()
        .await
        .map_err(|_| offline.to_string())?;
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err("The downloaded yt-dlp failed its checksum check, so it was discarded".into());
    }

    let dest = bins::managed_ytdlp(data_dir);
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

#[tauri::command]
pub async fn ytdlp_version(state: State<'_, AppState>) -> Result<String, String> {
    let path = bins::ytdlp(&state.data_dir);
    if !path.is_absolute() {
        return Err("not installed yet".into());
    }
    jobs::capture(&path, &["--version"])
        .await
        .map(|v| v.trim().to_string())
}

/// Update the writable yt-dlp copy in place and return its new version.
#[tauri::command]
pub async fn update_ytdlp(state: State<'_, AppState>) -> Result<String, String> {
    run_ytdlp_update(&state.data_dir).await
}

pub async fn run_ytdlp_update(data_dir: &std::path::Path) -> Result<String, String> {
    let path = ensure_ytdlp(data_dir).await?;
    if !bins::ytdlp_is_managed(data_dir) {
        return Err("This downloader can't be updated in place".into());
    }
    jobs::capture(&path, &["-U"]).await?;
    jobs::capture(&path, &["--version"])
        .await
        .map(|v| v.trim().to_string())
}
