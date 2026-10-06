//! Opening, previewing and exporting videos.

use fillerncut_core::export::{build_export_args, build_preview_args, export_duration};
use fillerncut_core::{parse_encoders, parse_ffprobe, Encoder, EncoderSupport, ExportSpec, MediaInfo};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime, State};

use crate::state::AppState;
use crate::{bins, jobs};

/// Let the webview load a local file through the `asset:` protocol.
pub fn allow_asset<R: Runtime>(app: &AppHandle<R>, path: &Path) {
    let _ = app.asset_protocol_scope().allow_file(path);
}

/// ffmpeg lists a GPU encoder whether or not this PC has the GPU, so each one is
/// tried on a tiny test clip; only those that really work are offered.
async fn gpu_encoder_works(encoder: Encoder) -> bool {
    jobs::capture(
        &bins::ffmpeg(),
        &[
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x256:r=10:d=0.5",
            "-frames:v",
            "3",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            encoder.ffmpeg_name(),
            "-f",
            "null",
            "-",
        ],
    )
    .await
    .is_ok()
}

async fn encoder_support(state: &AppState) -> EncoderSupport {
    *state
        .encoders
        .get_or_init(|| async {
            let mut support = jobs::capture(&bins::ffmpeg(), &["-hide_banner", "-encoders"])
                .await
                .map(|o| parse_encoders(&o))
                .unwrap_or_default();
            let (nvenc, qsv, amf) = tokio::join!(
                async { support.nvenc && gpu_encoder_works(Encoder::Nvenc).await },
                async { support.qsv && gpu_encoder_works(Encoder::Qsv).await },
                async { support.amf && gpu_encoder_works(Encoder::Amf).await },
            );
            support.nvenc = nvenc;
            support.qsv = qsv;
            support.amf = amf;
            support
        })
        .await
}

/// The encoder the user chose in Settings (`None` = Automatic).
fn preferred_encoder(state: &AppState) -> Option<Encoder> {
    Encoder::from_id(&state.settings().encoder)
}

/// Encoders to try, in order: the user's choice first (or the best available when
/// it's Automatic), the CPU last as a safety net unless the CPU was chosen.
///
/// `FILLERNCUT_ENCODER=videotoolbox` forces VideoToolbox to be tried first even
/// when it isn't detected (used by the fallback test); it never removes the
/// software fallback.
pub async fn encoder_candidates(state: &AppState) -> Vec<Encoder> {
    let mut support = encoder_support(state).await;
    if std::env::var("FILLERNCUT_ENCODER").is_ok_and(|v| v == "videotoolbox") {
        support.videotoolbox = true;
    }
    Encoder::candidates(&support, preferred_encoder(state))
}

pub async fn pick_encoder(state: &AppState) -> Result<Encoder, String> {
    encoder_candidates(state)
        .await
        .first()
        .copied()
        .ok_or_else(|| "ffmpeg has no H.264 encoder available".to_string())
}

#[derive(Serialize)]
pub struct EncoderOption {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    /// "windows", "macos" or "linux"
    pub platform: &'static str,
    /// The encoder exports will use first: an `Encoder::id()`, or "none".
    pub encoder: &'static str,
    /// Encoders that work on this machine, for the Settings picker.
    pub encoders: Vec<EncoderOption>,
    pub ffmpeg_found: bool,
}

#[tauri::command]
pub async fn app_info<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> Result<AppInfo, String> {
    let support = encoder_support(&state).await;
    let first = encoder_candidates(&state).await.first().copied();
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        platform: std::env::consts::OS,
        encoder: first.map_or("none", Encoder::id),
        encoders: Encoder::ALL
            .into_iter()
            .filter(|e| e.supported_by(&support))
            .map(|e| EncoderOption {
                id: e.id(),
                label: e.label(),
            })
            .collect(),
        ffmpeg_found: first.is_some(),
    })
}

#[tauri::command]
pub async fn probe_media<R: Runtime>(app: AppHandle<R>, path: String) -> Result<MediaInfo, String> {
    let p = Path::new(&path);
    if !p.is_file() {
        return Err("That file doesn't exist any more".into());
    }
    let json = jobs::capture(
        &bins::ffprobe(),
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            &path,
        ],
    )
    .await
    .map_err(|e| format!("Couldn't read that file as a video ({e})"))?;
    let mut info = parse_ffprobe(&json)?;
    if info.is_image {
        // Phone photos carry an EXIF rotation that ffmpeg applies but ffprobe doesn't
        // report. Edit in the orientation ffmpeg will actually produce.
        let report = jobs::capture_stderr(
            &bins::ffmpeg(),
            &[
                "-hide_banner",
                "-nostdin",
                "-i",
                &path,
                "-frames:v",
                "1",
                "-vf",
                "showinfo",
                "-f",
                "null",
                "-",
            ],
        )
        .await
        .unwrap_or_default();
        if let Some((w, h)) = fillerncut_core::parse_showinfo_size(&report) {
            info.width = w;
            info.height = h;
        }
    }
    allow_asset(&app, p);
    Ok(info)
}

fn preview_path(state: &AppState, input: &Path) -> PathBuf {
    let mut h = DefaultHasher::new();
    input.hash(&mut h);
    if let Ok(meta) = std::fs::metadata(input) {
        meta.len().hash(&mut h);
        if let Ok(m) = meta.modified() {
            m.hash(&mut h);
        }
    }
    state
        .cache_dir
        .join("previews")
        .join(format!("{:016x}.mp4", h.finish()))
}

/// Transcode a small H.264 proxy for files the webview can't play natively
/// (MKV, AVI, odd codecs). Cached by path + size + mtime.
#[tauri::command]
pub async fn make_preview<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
    has_audio: bool,
    duration: f64,
) -> Result<String, String> {
    let input = PathBuf::from(&path);
    let out = preview_path(&state, &input);
    if !out.is_file() {
        std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
        let encoder = pick_encoder(&state).await?;
        let guard = state.begin_job("preview")?;
        let partial = out.with_extension("partial.mp4");
        let args = build_preview_args(&path, &partial.to_string_lossy(), encoder, has_audio);
        let res = jobs::run_ffmpeg(&guard.token, &args, duration, |f| {
            jobs::emit_progress(&app, "preview", Some(f), None)
        })
        .await;
        if let Err(e) = res {
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
        std::fs::rename(&partial, &out).map_err(|e| e.to_string())?;
    }
    allow_asset(&app, &out);
    Ok(out.to_string_lossy().into_owned())
}

/// The folder finished exports go to (shown in Settings).
#[tauri::command]
pub fn export_dir<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> String {
    crate::dirs::finished_dir(&app, &state)
        .to_string_lossy()
        .into_owned()
}

/// A fresh timestamp-named path in the finished folder, e.g.
/// `~/Movies/FillernCut/Finished/2026-10-01_15-42-07.mp4` (or `.jpg` / `.png` for
/// photos). Never an existing file.
#[tauri::command]
pub fn default_save_path<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    ext: Option<String>,
) -> Result<String, String> {
    let ext = match ext.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("jpg") => "jpg",
        Some("png") => "png",
        _ => "mp4",
    };
    let dir = crate::dirs::finished_dir(&app, &state);
    crate::dirs::new_media_path(&dir, ext)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| format!("Can't create the folder {}: {e}", dir.display()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub video_bitrate: u64,
    pub bytes: u64,
}

/// Predicted output size for the current edit and quality (cheap; no encoding).
#[tauri::command]
pub fn estimate_export(spec: ExportSpec) -> Estimate {
    Estimate {
        video_bitrate: fillerncut_core::export::target_bitrate(&spec),
        bytes: fillerncut_core::export::estimate_bytes(&spec),
    }
}

#[tauri::command]
pub async fn export_video<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    mut spec: ExportSpec,
) -> Result<String, String> {
    if !Path::new(&spec.input).is_file() {
        return Err("The source file is missing".into());
    }
    for wm in &spec.watermarks {
        if !Path::new(&wm.path).is_file() {
            return Err("A watermark image is missing. Remove it from the video and add it again".into());
        }
    }
    let final_out = PathBuf::from(&spec.output);
    if let Some(parent) = final_out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Can't create the folder {}: {e}", parent.display()))?;
    }
    // Videos are .mp4; photos are .jpg / .png, matching the chosen format.
    let (want, want_alt) = match spec.image_format {
        None => ("mp4", "mp4"),
        Some(f) => (
            f.extension(),
            if f.extension() == "jpg" {
                "jpeg"
            } else {
                f.extension()
            },
        ),
    };
    let have = final_out
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if have.as_deref() != Some(want) && have.as_deref() != Some(want_alt) {
        return Err(format!("Export path must end in .{want}"));
    }
    if final_out == Path::new(&spec.input) {
        return Err("Choose a different file name than the source".into());
    }

    // Photos use ffmpeg's built-in JPEG/PNG encoders; only video needs an H.264 encoder.
    let candidates = if spec.image_format.is_some() {
        vec![fillerncut_core::Encoder::Libx264] // unused for photos
    } else {
        encoder_candidates(&state).await
    };
    if candidates.is_empty() {
        return Err("ffmpeg has no H.264 encoder available".into());
    }
    let total = export_duration(&spec);
    // Write next to the target, then rename, so a cancelled or failed export
    // never leaves a half-written file under the final name.
    let out_ext = final_out
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or(want)
        .to_string();
    let partial = final_out.with_extension(format!("partial.{out_ext}"));
    spec.output = partial.to_string_lossy().into_owned();

    let guard = state.begin_job("export")?;
    let mut last_error = String::new();
    for (attempt, encoder) in candidates.iter().enumerate() {
        let args = build_export_args(&spec, *encoder).map_err(|e| e.to_string())?;
        // After a hardware-encoder failure (unsupported size, busy media engine,
        // an Intel Mac...) retry on the CPU instead of giving up.
        let note = (attempt > 0).then_some("That encoder didn't work, exporting on the CPU…");
        let res = jobs::run_ffmpeg(&guard.token, &args, total, |f| {
            jobs::emit_progress(&app, "export", Some(f), note)
        })
        .await;
        match res {
            Ok(()) => {
                std::fs::rename(&partial, &final_out).map_err(|e| e.to_string())?;
                crate::history::record(
                    &state,
                    fillerncut_core::HistoryKind::Export,
                    &final_out,
                    fillerncut_core::NewEntry {
                        duration: (total > 0.0).then_some(total),
                        ..Default::default()
                    },
                )
                .await;
                return Ok(final_out.to_string_lossy().into_owned());
            }
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                if e == jobs::CANCELLED {
                    return Err(e);
                }
                last_error = e;
            }
        }
    }
    Err(last_error)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job: String) {
    state.cancel_job(&job);
}

/// Delete cached previews older than a few days.
pub fn prune_preview_cache(cache_dir: &Path) {
    let dir = cache_dir.join("previews");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 3600);
    for e in entries.flatten() {
        if let Ok(modified) = e.metadata().and_then(|m| m.modified()) {
            if modified < cutoff {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").args(["-R", &path]).status();
    #[cfg(windows)]
    let status = {
        // `explorer /select,<path>` highlights the file. Explorer exits with 1 even on
        // success, so only a failure to start it is an error.
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.replace('/', "\\")))
            .status()
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let status = std::process::Command::new("xdg-open")
        .arg(Path::new(&path).parent().unwrap_or(Path::new(".")))
        .status();
    status.map(|_| ()).map_err(|e| e.to_string())
}

/// Open a link in the default browser. Only this project's GitHub pages are
/// allowed, since the only links in the UI are release notes and the repo.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    const ALLOWED: &str = "https://github.com/filipohano/videocut";
    if url != ALLOWED && !url.starts_with(&format!("{ALLOWED}/")) {
        return Err("That link isn't allowed".into());
    }
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(&url).status();
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::CommandExt;
        // `start` is a cmd builtin; the empty "" is the window title it would otherwise eat.
        std::process::Command::new("cmd")
            .raw_arg(format!("/c start \"\" \"{url}\""))
            .creation_flags(0x0800_0000)
            .status()
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let status = std::process::Command::new("xdg-open").arg(&url).status();
    status.map(|_| ()).map_err(|e| e.to_string())
}
