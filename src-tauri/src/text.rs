//! Text on the video: the pictures of text boxes (drawn in the page, burned in by ffmpeg) and
//! the saved text styles.

use fillerncut_core::{TextPreset, TextPresetStyle};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tauri::State;

use crate::state::AppState;

/// A text picture is a few hundred kB; this is far beyond any real one.
const MAX_PNG_BYTES: usize = 40 * 1024 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Where the pictures of text boxes wait for ffmpeg. They only live for one export, so this is in
/// the cache and is emptied at every launch.
pub fn text_images_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("texts")
}

pub fn clear_text_images(cache_dir: &Path) {
    let _ = std::fs::remove_dir_all(text_images_dir(cache_dir));
}

/// Save the PNG of a text box for the exporter, and say where it is. The name is a hash of the
/// picture, so the same text exported twice uses one file.
#[tauri::command]
pub fn save_text_image(state: State<'_, AppState>, png_base64: String) -> Result<String, String> {
    use base64::Engine;
    if png_base64.len() > MAX_PNG_BYTES * 4 / 3 + 8 {
        return Err("That text picture is too large".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .map_err(|_| "The text picture is damaged".to_string())?;
    if bytes.len() > MAX_PNG_BYTES || !bytes.starts_with(PNG_SIGNATURE) {
        return Err("The text picture isn't a PNG".into());
    }
    let dir = text_images_dir(&state.cache_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't create the folder for text pictures: {e}"))?;
    let path = dir.join(format!("{:x}.png", Sha256::digest(&bytes)));
    if !path.is_file() {
        // Written under another name first, so ffmpeg never reads half a file.
        let partial = path.with_extension("part");
        std::fs::write(&partial, &bytes).map_err(|e| format!("Couldn't save the text picture: {e}"))?;
        std::fs::rename(&partial, &path).map_err(|e| format!("Couldn't save the text picture: {e}"))?;
    }
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn text_presets_list(state: State<'_, AppState>) -> Vec<TextPreset> {
    state.text_presets.list()
}

#[tauri::command]
pub fn text_presets_save(
    state: State<'_, AppState>,
    name: String,
    style: TextPresetStyle,
) -> Result<TextPreset, String> {
    state.text_presets.save(&name, style).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn text_presets_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.text_presets.remove(&id).map_err(|e| e.to_string())
}
