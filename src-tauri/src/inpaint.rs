//! "Remove text": filling in part of a still picture with the MI-GAN model (downloaded as an
//! add-on), run by tract, a pure-Rust ONNX engine, so nothing native has to be shipped.
//! The maths around the model is in `fillerncut_core::inpaint`.

use fillerncut_core::inpaint::{
    apply_patch, build_mask, classify, fill, freezedetect_args, parse_freezes, PixelRect, Stillness,
    MODEL_SIZE,
};
use fillerncut_core::FillPatch;
use image::{GrayImage, ImageFormat, Rgb, RgbImage};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Runtime, State};
use tokio::process::Command;
use tract_onnx::prelude::*;

use crate::modules::{installed_file, REMOVE_TEXT};
use crate::state::AppState;
use crate::{bins, editor, jobs};

type Model = Arc<TypedRunnableModel>;

/// The loaded model (loading takes a moment, so it is kept), with the file it came from.
static MODEL: Mutex<Option<(PathBuf, Model)>> = Mutex::new(None);

/// Drop the loaded model (the add-on was removed).
pub fn forget_model() {
    *MODEL.lock().unwrap() = None;
}

fn load_model(path: &Path) -> Result<Model, String> {
    if let Some((loaded, model)) = MODEL.lock().unwrap().as_ref() {
        if loaded == path {
            return Ok(model.clone());
        }
    }
    // Use every core for the heavy maths.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    tract_linalg::multithread::set_default_executor(tract_linalg::multithread::Executor::multithread(
        threads,
    ));
    let size = MODEL_SIZE as usize;
    let model = tract_onnx::onnx()
        .model_for_path(path)
        .and_then(|m| m.with_input_fact(0, u8::fact([1, 3, size, size]).into()))
        .and_then(|m| m.with_input_fact(1, u8::fact([1, 1, size, size]).into()))
        .and_then(|m| m.into_optimized())
        .and_then(|m| m.into_runnable())
        .map_err(|e| {
            format!("The “Remove text” add-on couldn't be loaded ({e}). Remove it in Settings → Add-ons and download it again.")
        })?;
    *MODEL.lock().unwrap() = Some((path.to_path_buf(), model.clone()));
    Ok(model)
}

/// One run of the model: a square picture and its "keep" mask in, the filled picture out.
fn run_model(model: &Model, picture: &RgbImage, keep: &GrayImage) -> Result<RgbImage, String> {
    let (w, h) = picture.dimensions();
    let plane = (w * h) as usize;
    let mut planar = vec![0u8; plane * 3];
    for (i, p) in picture.pixels().enumerate() {
        for c in 0..3 {
            planar[c * plane + i] = p.0[c];
        }
    }
    let image = Tensor::from_shape(&[1, 3, h as usize, w as usize], &planar).map_err(|e| e.to_string())?;
    let mask =
        Tensor::from_shape(&[1, 1, h as usize, w as usize], keep.as_raw()).map_err(|e| e.to_string())?;
    let out = model
        .run(tvec!(image.into(), mask.into()))
        .map_err(|e| e.to_string())?;
    let view = out[0].to_plain_array_view::<u8>().map_err(|e| e.to_string())?;
    if view.shape() != [1, 3, h as usize, w as usize] {
        return Err(format!("it returned a picture shaped {:?}", view.shape()));
    }
    let data: Vec<u8> = view.iter().copied().collect();
    Ok(RgbImage::from_fn(w, h, |x, y| {
        let i = (y * w + x) as usize;
        Rgb([data[i], data[plane + i], data[2 * plane + i]])
    }))
}

/// One frame of a video (or the photo itself) as ffmpeg decodes it: rotation applied, exactly
/// what the exporter will work on.
async fn grab_frame(path: &str, time: Option<f64>) -> Result<RgbImage, String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
    ];
    if let Some(t) = time.filter(|t| t.is_finite() && *t > 0.0) {
        args.extend(["-ss".into(), format!("{t:.3}")]);
    }
    args.extend(
        [
            "-i",
            path,
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "-",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let out = bins::hide_window(&mut Command::new(bins::ffmpeg()))
        .args(&args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("Couldn't start ffmpeg: {e}"))?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(format!(
            "Couldn't read the picture: {}",
            jobs::summarize_ffmpeg_error(&String::from_utf8_lossy(&out.stderr))
        ));
    }
    image::load_from_memory_with_format(&out.stdout, ImageFormat::Png)
        .map(|i| i.to_rgb8())
        .map_err(|e| format!("Couldn't read the picture: {e}"))
}

/// Where filled-in areas wait for the exporter. Emptied at every launch.
pub fn fills_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("fills")
}

pub fn clear_fills(cache_dir: &Path) {
    let _ = std::fs::remove_dir_all(fills_dir(cache_dir));
}

/// Whether a video's picture stands still (and if it is a slideshow, where each photo is).
#[tauri::command]
pub async fn analyze_stills(path: String, duration: f64) -> Result<Stillness, String> {
    let args = freezedetect_args(&path);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let stderr = jobs::capture_stderr(&bins::ffmpeg(), &args).await?;
    Ok(classify(&parse_freezes(&stderr, duration), duration))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FillResult {
    /// The patch (PNG with transparency) to lay over the uncropped frame.
    path: String,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// No caption letters were found in the box, so all of it was filled.
    whole_box: bool,
}

/// Fill in the text inside `boxes` (pixels of the full frame) on the picture at `time`.
/// `under` are areas already filled on this picture: the new fill is made on top of them.
#[tauri::command]
pub async fn inpaint_fill<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
    time: Option<f64>,
    boxes: Vec<PixelRect>,
    letters: bool,
    under: Vec<FillPatch>,
) -> Result<FillResult, String> {
    let model_path = installed_file(&state.data_dir, &REMOVE_TEXT)
        .ok_or("Download “Remove text (AI)” in Settings → Add-ons first.")?;
    let _guard = state
        .begin_job("inpaint")
        .map_err(|_| "Text is already being removed. Wait for it to finish.".to_string())?;

    let mut frame = grab_frame(&path, time).await?;
    for patch in &under {
        let img = image::open(&patch.path)
            .map_err(|e| format!("An earlier fill is missing ({e}). Undo it and try again."))?
            .to_rgba8();
        apply_patch(&mut frame, &img, patch.x as i64, patch.y as i64);
    }

    let (patch, whole_box) = tokio::task::spawn_blocking(move || {
        let (mask, whole_box) = build_mask(&frame, &boxes, letters).map_err(|e| e.to_string())?;
        let model = load_model(&model_path)?;
        let patch =
            fill(&frame, &mask, |pic, keep| run_model(&model, pic, keep)).map_err(|e| e.to_string())?;
        Ok::<_, String>((patch, whole_box))
    })
    .await
    .map_err(|e| format!("Removing the text stopped unexpectedly: {e}"))??;

    let mut png = Vec::new();
    patch
        .image
        .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let dir = fills_dir(&state.cache_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't save the result: {e}"))?;
    let file = dir.join(format!("{:x}.png", Sha256::digest(&png)));
    if !file.is_file() {
        let partial = file.with_extension("part");
        std::fs::write(&partial, &png).map_err(|e| format!("Couldn't save the result: {e}"))?;
        std::fs::rename(&partial, &file).map_err(|e| format!("Couldn't save the result: {e}"))?;
    }
    editor::allow_asset(&app, &file);
    Ok(FillResult {
        path: file.to_string_lossy().into_owned(),
        x: patch.x,
        y: patch.y,
        w: patch.image.width(),
        h: patch.image.height(),
        whole_box,
    })
}
