//! Add-ons: parts of the app that are downloaded only by people who want them (Settings →
//! Add-ons), so the app itself stays small. Each one is a file from a fixed address, checked
//! against a SHA-256 written into the app, kept in the app data folder.

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime, State};
use tokio::io::AsyncWriteExt;

use crate::download::http_client;
use crate::jobs::{emit_progress, CANCELLED};
use crate::state::AppState;

pub struct ModuleSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub file_name: &'static str,
    /// A fixed revision, so the file can't change under the checksum.
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// "Remove text": MI-GAN, an image-inpainting model by Picsart AI Research (MIT license).
pub const REMOVE_TEXT: ModuleSpec = ModuleSpec {
    id: "remove-text",
    name: "Remove text (AI)",
    description: "Removes captions and other text from photos and from videos where the picture stands still (photo posts with music, slideshows), and fills in what was behind them. Runs on your computer; nothing is uploaded. Uses MI-GAN by Picsart AI Research (MIT license).",
    file_name: "migan.onnx",
    url: "https://huggingface.co/andraniksargsyan/migan/resolve/406830d0fa60666da0071c342ad2fbc8f30c5c64/migan.onnx",
    sha256: "593eba0b7e04730f1b61c0a3cbca68d97d8d6a7ff5c6a44a7b9d7fcd880fc5ae",
    bytes: 29_546_882,
};

pub const MODULES: &[&ModuleSpec] = &[&REMOVE_TEXT];

fn spec(id: &str) -> Result<&'static ModuleSpec, String> {
    MODULES
        .iter()
        .copied()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("There is no add-on called “{id}”"))
}

/// The job name its download runs under (the page cancels it by this name).
pub fn job(id: &str) -> String {
    format!("module-{id}")
}

pub fn module_dir(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("modules").join(id)
}

/// Where a module's file is, once it has been downloaded.
pub fn installed_file(data_dir: &Path, spec: &ModuleSpec) -> Option<PathBuf> {
    let path = module_dir(data_dir, spec.id).join(spec.file_name);
    path.is_file().then_some(path)
}

/// The address and checksum to download. The integration tests point these at a local server.
fn source(spec: &ModuleSpec) -> (String, String) {
    match (
        std::env::var("FILLERNCUT_MODULE_URL"),
        std::env::var("FILLERNCUT_MODULE_SHA256"),
    ) {
        (Ok(url), Ok(sha)) => (url, sha),
        _ => (spec.url.to_string(), spec.sha256.to_string()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleView {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    bytes: u64,
    installed: bool,
}

fn view(data_dir: &Path, spec: &'static ModuleSpec) -> ModuleView {
    ModuleView {
        id: spec.id,
        name: spec.name,
        description: spec.description,
        bytes: spec.bytes,
        installed: installed_file(data_dir, spec).is_some(),
    }
}

#[tauri::command]
pub fn modules_list(state: State<'_, AppState>) -> Vec<ModuleView> {
    MODULES.iter().map(|m| view(&state.data_dir, m)).collect()
}

/// Download an add-on, with progress under the job `module-<id>`. The file only appears under
/// its real name once it is complete and its checksum matches.
#[tauri::command]
pub async fn module_install<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: String,
) -> Result<ModuleView, String> {
    let spec = spec(&id)?;
    let job = job(spec.id);
    let guard = state
        .begin_job(&job)
        .map_err(|_| format!("{} is already being downloaded", spec.name))?;
    let dir = module_dir(&state.data_dir, spec.id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("Couldn't create a folder for the add-on: {e}"))?;
    let dest = dir.join(spec.file_name);
    let partial = dir.join(format!("{}.part", spec.file_name));
    let (url, sha256) = source(spec);

    emit_progress(&app, &job, Some(0.0), Some("Downloading…"));
    let result = async {
        let response = http_client()?
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Couldn't download {}: {e}", spec.name))?;
        let total = response.content_length().filter(|t| *t > 0).unwrap_or(spec.bytes);
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|e| format!("Couldn't save the add-on: {e}"))?;
        let mut hasher = Sha256::new();
        let mut done = 0u64;
        loop {
            let chunk = tokio::select! {
                _ = guard.token.cancelled() => return Err(CANCELLED.to_string()),
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| format!("The download was interrupted: {e}"))?;
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Couldn't save the add-on: {e}"))?;
            done += chunk.len() as u64;
            emit_progress(
                &app,
                &job,
                Some((done as f64 / total as f64).clamp(0.0, 1.0)),
                Some("Downloading…"),
            );
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        if format!("{:x}", hasher.finalize()) != sha256.to_ascii_lowercase() {
            return Err(format!(
                "The downloaded {} wasn't the expected file (its checksum doesn't match), so it wasn't installed. Try again later.",
                spec.name
            ));
        }
        tokio::fs::rename(&partial, &dest)
            .await
            .map_err(|e| format!("Couldn't save the add-on: {e}"))
    }
    .await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(e);
    }
    emit_progress(&app, &job, Some(1.0), None);
    Ok(view(&state.data_dir, spec))
}

/// Delete a downloaded add-on. Its features lock again.
#[tauri::command]
pub fn module_remove(state: State<'_, AppState>, id: String) -> Result<ModuleView, String> {
    let spec = spec(&id)?;
    if spec.id == REMOVE_TEXT.id {
        crate::inpaint::forget_model();
    }
    let dir = module_dir(&state.data_dir, spec.id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("Couldn't delete the add-on: {e}"))?;
    }
    Ok(view(&state.data_dir, spec))
}
