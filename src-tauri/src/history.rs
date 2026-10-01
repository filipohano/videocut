//! History of downloads and exports.

use fillerncut_core::history::{build_thumbnail_args, HistoryEntry, HistoryKind, NewEntry};
use std::path::Path;
use tauri::State;

use crate::state::AppState;
use crate::{bins, jobs};

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Remember a finished download/export, with a small preview image. Never fails
/// the caller: history is a convenience.
pub async fn record(state: &AppState, kind: HistoryKind, path: &Path, mut new: NewEntry) {
    new.path = path.to_string_lossy().into_owned();
    new.bytes = std::fs::metadata(path).ok().map(|m| m.len());
    let Ok(entry) = state.history.add(kind, new, now_unix()) else {
        return;
    };

    let _ = std::fs::create_dir_all(state.history.thumbs_dir());
    let out = state.history.thumb_path(&entry.id);
    let (input, output) = (
        path.to_string_lossy().into_owned(),
        out.to_string_lossy().into_owned(),
    );
    // A frame just after the start; very short clips need the very first frame.
    for at in [0.4, 0.0] {
        let args = build_thumbnail_args(&input, &output, at);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        if jobs::capture(&bins::ffmpeg(), &args).await.is_ok() && out.is_file() {
            let _ = state.history.mark_thumb(&entry.id);
            break;
        }
    }
}

#[tauri::command]
pub fn history_list(state: State<'_, AppState>) -> Vec<HistoryEntry> {
    state.history.list()
}

#[tauri::command]
pub fn history_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.history.remove(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_clear(state: State<'_, AppState>) -> Result<(), String> {
    state.history.clear().map_err(|e| e.to_string())
}
