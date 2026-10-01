//! Watermark library commands. The library itself lives in `fillerncut-core`.

use fillerncut_core::{WatermarkEntry, WatermarkPatch};
use std::path::Path;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub async fn library_list(state: State<'_, AppState>) -> Result<Vec<WatermarkEntry>, String> {
    Ok(state.library.list())
}

#[tauri::command]
pub async fn library_add(state: State<'_, AppState>, path: String) -> Result<WatermarkEntry, String> {
    state
        .library
        .add_from_path(Path::new(&path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn library_update(
    state: State<'_, AppState>,
    id: String,
    patch: WatermarkPatch,
) -> Result<WatermarkEntry, String> {
    state.library.update(&id, &patch).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn library_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.library.remove(&id).map_err(|e| e.to_string())
}
