//! Watermark library commands. The library itself lives in `fillerncut-core`.

use base64::Engine;
use fillerncut_core::{TextStyle, WatermarkEntry, WatermarkPatch};
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

fn decode_png(b64: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| "The text image was corrupted".to_string())
}

/// Save a text watermark. The UI renders the text to a PNG (so any installed
/// font works, and preview == export) and sends it with the style.
#[tauri::command]
pub async fn library_add_text(
    state: State<'_, AppState>,
    png_base64: String,
    style: TextStyle,
) -> Result<WatermarkEntry, String> {
    let png = decode_png(&png_base64)?;
    state.library.add_text(&png, style).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn library_replace_text(
    state: State<'_, AppState>,
    id: String,
    png_base64: String,
    style: TextStyle,
) -> Result<WatermarkEntry, String> {
    let png = decode_png(&png_base64)?;
    state
        .library
        .replace_text(&id, &png, style)
        .map_err(|e| e.to_string())
}
