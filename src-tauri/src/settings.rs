//! Settings commands and yt-dlp maintenance.

use fillerncut_core::Settings;
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

#[tauri::command]
pub async fn ytdlp_version(state: State<'_, AppState>) -> Result<String, String> {
    let path = bins::ytdlp(&state.data_dir);
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
    let path = bins::ytdlp(data_dir);
    if !bins::ytdlp_is_managed(data_dir) {
        return Err("The bundled downloader can't be updated in place".into());
    }
    jobs::capture(&path, &["-U"]).await?;
    jobs::capture(&path, &["--version"])
        .await
        .map(|v| v.trim().to_string())
}
